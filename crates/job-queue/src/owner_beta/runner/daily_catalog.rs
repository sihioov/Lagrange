//! Private navigation only. No record or catalogue is an approval capability.
use super::daily_ingest::IngestError;
use domain::{ContentHash, TradingDate};
use market_data::daily_price_only::{
    DailyPricePinsV1Wire,
    artifact::{
        CapturedDailyArtifactNames, DailyAdmissionRoot, DailyArtifactNameLimits,
        DailyCandidateRoot, DailySourceFrontierRoot,
    },
    frontier::read_daily_frontier_navigation,
    replay::read_daily_admission_pins,
};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

struct Node {
    hash: ContentHash,
    admission: ContentHash,
    parent: Option<ContentHash>,
    start: TradingDate,
    end: TradingDate,
}
type NextAdmission<'a> = (&'a DailyPricePinsV1Wire, Option<&'a ContentHash>);
pub(super) struct Catalog {
    captured_admissions: CapturedDailyArtifactNames,
    captured_frontiers: CapturedDailyArtifactNames,
    captured_candidates: CapturedDailyArtifactNames,
    admissions: BTreeMap<ContentHash, DailyPricePinsV1Wire>,
    chain: Vec<Node>,
    incomplete: Option<ContentHash>,
    unadmitted_candidate: Option<ContentHash>,
}
impl Catalog {
    pub(super) fn read(
        admissions: &DailyAdmissionRoot,
        frontiers: &DailySourceFrontierRoot,
        candidates: &DailyCandidateRoot,
        limits: DailyArtifactNameLimits,
        owner: Uuid,
        policy: &ContentHash,
        mut keep_reading: impl FnMut() -> bool,
    ) -> Result<Self, IngestError> {
        let checkpoint = |keep: &mut dyn FnMut() -> bool| {
            if keep() {
                Ok(())
            } else {
                Err(IngestError::Interrupted)
            }
        };
        checkpoint(&mut keep_reading)?;
        let captured_admissions = admissions
            .capture_names(limits)
            .map_err(IngestError::artifact)?;
        let captured_frontiers = frontiers
            .capture_names(limits)
            .map_err(IngestError::artifact)?;
        let captured_candidates = candidates
            .capture_names(limits)
            .map_err(IngestError::artifact)?;
        let mut pins = BTreeMap::new();
        let mut candidate_contents = BTreeSet::new();
        for hash in captured_admissions.names() {
            checkpoint(&mut keep_reading)?;
            let value = read_daily_admission_pins(admissions, hash, policy)
                .map_err(|_| IngestError::InvalidArtifact)?;
            if value.policy_sha256 != *policy
                || !candidate_contents.insert(value.content_sha256.clone())
            {
                return Err(IngestError::Conflict);
            }
            pins.insert(hash.clone(), value);
        }
        let candidate_set: BTreeSet<_> = captured_candidates.names().iter().cloned().collect();
        if !candidate_contents.is_subset(&candidate_set) {
            return Err(IngestError::InvalidArtifact);
        }
        let mut nodes = Vec::new();
        for hash in captured_frontiers.names() {
            checkpoint(&mut keep_reading)?;
            let nav = read_daily_frontier_navigation(frontiers, hash)
                .map_err(|_| IngestError::InvalidArtifact)?;
            if nav.owner() != owner || nav.policy_sha256() != policy {
                return Err(IngestError::InvalidArtifact);
            }
            nodes.push(Node {
                hash: hash.clone(),
                admission: nav.admission_sha256().clone(),
                parent: nav.parent_frontier_sha256().cloned(),
                start: nav.range_start(),
                end: nav.range_end(),
            });
        }
        let chain = plan_chain(nodes, &pins)?;
        let covered: BTreeSet<_> = chain.iter().map(|node| &node.admission).collect();
        let mut incomplete = pins.keys().filter(|hash| !covered.contains(hash));
        let incomplete_first = incomplete.next().cloned();
        if incomplete.next().is_some() {
            return Err(IngestError::Conflict);
        }
        let mut new_candidates = candidate_set.difference(&candidate_contents);
        let unadmitted_candidate = new_candidates.next().cloned();
        if new_candidates.next().is_some() {
            return Err(IngestError::Conflict);
        }
        let value = Self {
            captured_admissions,
            captured_frontiers,
            captured_candidates,
            admissions: pins,
            chain,
            incomplete: incomplete_first,
            unadmitted_candidate,
        };
        value.revalidate()?;
        checkpoint(&mut keep_reading)?;
        Ok(value)
    }

    pub(super) fn revalidate(&self) -> Result<(), IngestError> {
        self.captured_candidates
            .revalidate()
            .map_err(IngestError::artifact)?;
        self.captured_admissions
            .revalidate()
            .map_err(IngestError::artifact)?;
        self.captured_frontiers
            .revalidate()
            .map_err(IngestError::artifact)
    }

    pub(super) fn tip(&self) -> Option<(&DailyPricePinsV1Wire, &ContentHash)> {
        self.chain.last().and_then(|node| {
            self.admissions
                .get(&node.admission)
                .map(|pins| (pins, &node.hash))
        })
    }

    pub(super) fn incomplete(&self) -> Option<&DailyPricePinsV1Wire> {
        self.incomplete
            .as_ref()
            .and_then(|hash| self.admissions.get(hash))
    }

    pub(super) fn unadmitted_candidate(&self) -> Option<&ContentHash> {
        self.unadmitted_candidate.as_ref()
    }

    pub(super) fn has_candidate(&self, content: &ContentHash, manifest: &ContentHash) -> bool {
        self.chain.iter().any(|node| {
            self.admissions.get(&node.admission).is_some_and(|pins| {
                &pins.content_sha256 == content && &pins.manifest_sha256 == manifest
            })
        })
    }

    /// SQL supplies the current committed admission. Return at most one next
    /// physical orphan; a larger divergence cannot be skipped or guessed.
    pub(super) fn next_after(
        &self,
        current: Option<&ContentHash>,
    ) -> Result<Option<NextAdmission<'_>>, IngestError> {
        let next = match current {
            None => 0,
            Some(hash) => {
                self.chain
                    .iter()
                    .position(|node| &node.admission == hash)
                    .ok_or(IngestError::InvalidArtifact)?
                    + 1
            }
        };
        let beyond = self
            .chain
            .len()
            .checked_sub(next)
            .ok_or(IngestError::InvalidArtifact)?;
        if beyond + usize::from(self.incomplete.is_some()) > 1 {
            return Err(IngestError::Conflict);
        }
        if let Some(node) = self.chain.get(next) {
            return self
                .admissions
                .get(&node.admission)
                .map(|pins| Some((pins, Some(&node.hash))))
                .ok_or(IngestError::InvalidArtifact);
        }
        Ok(self.incomplete().map(|pins| (pins, None)))
    }

    pub(super) fn current_pair(
        &self,
        current: &ContentHash,
    ) -> Result<(&DailyPricePinsV1Wire, &ContentHash), IngestError> {
        let node = self
            .chain
            .iter()
            .find(|node| &node.admission == current)
            .ok_or(IngestError::InvalidArtifact)?;
        self.admissions
            .get(current)
            .map(|pins| (pins, &node.hash))
            .ok_or(IngestError::InvalidArtifact)
    }
}

fn plan_chain(
    nodes: Vec<Node>,
    admissions: &BTreeMap<ContentHash, DailyPricePinsV1Wire>,
) -> Result<Vec<Node>, IngestError> {
    if nodes.is_empty() {
        return Ok(Vec::new());
    }
    let mut by_hash = BTreeMap::new();
    let mut child = BTreeMap::new();
    let mut root = None;
    let mut covered = BTreeSet::new();
    let length = nodes.len();
    for node in nodes {
        if !admissions.contains_key(&node.admission)
            || !covered.insert(node.admission.clone())
            || node.end < node.start
            || by_hash.contains_key(&node.hash)
        {
            return Err(IngestError::Conflict);
        }
        if let Some(parent) = &node.parent {
            if child.insert(parent.clone(), node.hash.clone()).is_some() {
                return Err(IngestError::Conflict);
            }
        } else if root.replace(node.hash.clone()).is_some() {
            return Err(IngestError::Conflict);
        }
        by_hash.insert(node.hash.clone(), node);
    }
    let mut hash = root.ok_or(IngestError::Conflict)?;
    let mut ordered: Vec<Node> = Vec::new();
    loop {
        let node = by_hash.remove(&hash).ok_or(IngestError::Conflict)?;
        if ordered.last().is_some_and(|parent| {
            node.parent.as_ref() != Some(&parent.hash)
                || node.start != parent.start
                || node.end <= parent.end
        }) {
            return Err(IngestError::Conflict);
        }
        let next = child.get(&hash).cloned();
        ordered.push(node);
        match next {
            None => break,
            Some(value) => hash = value,
        }
    }
    if ordered.len() != length || !by_hash.is_empty() {
        return Err(IngestError::Conflict);
    }
    Ok(ordered)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hash(value: u8) -> ContentHash {
        ContentHash::from_bytes(&[value])
    }
    fn day(value: &str) -> TradingDate {
        TradingDate::parse(value).unwrap()
    }
    fn node(id: u8, parent: Option<u8>, end: &str) -> Node {
        Node {
            hash: hash(id),
            admission: hash(id + 10),
            parent: parent.map(hash),
            start: day("2016-08-29"),
            end: day(end),
        }
    }
    fn pins() -> BTreeMap<ContentHash, DailyPricePinsV1Wire> {
        (1..=4)
            .map(|id| {
                (
                    hash(id + 10),
                    DailyPricePinsV1Wire {
                        content_sha256: hash(id + 20),
                        manifest_sha256: hash(id + 30),
                        admission_sha256: hash(id + 10),
                        policy_sha256: hash(99),
                    },
                )
            })
            .collect()
    }
    #[test]
    fn daily_catalog_orders_only_one_complete_parent_chain() {
        let chain = plan_chain(
            vec![
                node(3, Some(2), "2026-09-02"),
                node(1, None, "2026-08-31"),
                node(2, Some(1), "2026-09-01"),
            ],
            &pins(),
        )
        .unwrap();
        assert_eq!(
            chain.iter().map(|n| n.hash.clone()).collect::<Vec<_>>(),
            vec![hash(1), hash(2), hash(3)]
        );
        assert!(plan_chain(vec![], &BTreeMap::new()).unwrap().is_empty());
    }
    #[test]
    fn daily_catalog_rejects_forks_duplicate_admissions_and_disconnected_roots() {
        for nodes in [
            vec![
                node(1, None, "2026-08-31"),
                node(2, Some(1), "2026-09-01"),
                node(3, Some(1), "2026-09-02"),
            ],
            vec![node(1, None, "2026-08-31"), node(2, None, "2026-09-01")],
            vec![node(1, None, "2026-08-31"), node(1, Some(1), "2026-09-01")],
        ] {
            assert!(matches!(
                plan_chain(nodes, &pins()),
                Err(IngestError::Conflict)
            ));
        }
        let mut second = node(2, Some(1), "2026-09-01");
        second.admission = hash(11);
        assert!(plan_chain(vec![node(1, None, "2026-08-31"), second], &pins()).is_err());
    }
    #[test]
    fn daily_catalog_rejects_cycles_missing_parents_and_unknown_admissions() {
        for nodes in [
            vec![
                node(1, Some(2), "2026-08-31"),
                node(2, Some(1), "2026-09-01"),
            ],
            vec![node(1, None, "2026-08-31"), node(2, Some(4), "2026-09-01")],
            vec![
                node(1, None, "2026-08-31"),
                node(2, Some(3), "2026-09-01"),
                node(3, Some(2), "2026-09-02"),
            ],
        ] {
            assert!(plan_chain(nodes, &pins()).is_err());
        }
        assert!(plan_chain(vec![node(1, None, "2026-08-31")], &BTreeMap::new()).is_err());
    }
    #[test]
    fn daily_catalog_cannot_advance_across_inconsistent_or_regressing_ranges() {
        for end in ["2026-08-28", "2026-08-31"] {
            assert!(
                plan_chain(
                    vec![node(1, None, "2026-08-31"), node(2, Some(1), end)],
                    &pins()
                )
                .is_err()
            );
        }
        let mut second = node(2, Some(1), "2026-09-01");
        second.start = day("2016-08-30");
        assert!(plan_chain(vec![node(1, None, "2026-08-31"), second], &pins()).is_err());
    }
}
