//! The strategy registry (design §6.7, FR-STR-001..005).
//!
//! Every strategy package is versioned (`strategy_id` + immutable SemVer) and
//! enters the registry in [`StrategyState::Draft`].  Published versions are
//! **immutable**: re-registering an existing (id, version) is a typed
//! [`RegistryError::ImmutableVersion`] denial, and old runs always resolve the
//! ORIGINAL definition of a version even after a new release is published.
//!
//! States: `Draft | Validated | Paper | LiveCandidate | Retired`, reached
//! only through the documented promotion gates (FR-STR-003):
//!
//! | Transition | Required evidence |
//! | --- | --- |
//! | Draft -> Validated | golden + holdout + cost checks (all manifests non-empty) |
//! | Validated -> Paper | parity report + minimum observation window (>= 21 sessions) |
//! | Paper -> LiveCandidate | Phase 3 safety bundle covering all documented checks |
//! | any -> Retired | Owner only; terminal |
//!
//! **Authorization**: registering, promoting, retiring, and code deployment
//! are Owner-only.  Members may change ONLY schema-bound parameters of an
//! existing version ([`Registry::apply_member_config`]); arbitrary Member
//! code upload is denied with [`RegistryError::MemberCodeDenied`].  Every
//! operation — approved or denied — is appended to the audit log
//! ([`Registry::audit`]), which is append-only by construction.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

use domain::{ContentHash, StrategyVersion};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

/// The minimum Paper observation window in sessions (a calendar month of KRX
/// sessions).  Shorter windows are a typed denial.
pub const MIN_PAPER_OBSERVATION_SESSIONS: u64 = 21;

/// The documented Phase 3 safety evidence checklist (design §11, FR-LIVE):
/// every check must be covered by a LiveCandidate safety bundle.
pub const PHASE3_SAFETY_CHECKS: &[&str] = &[
    "kill_switch",
    "fail_closed_restart",
    "startup_reconciliation",
    "runtime_reconciliation",
    "credential_references",
    "rate_limits",
    "risk_gatekeeper",
    "idempotent_order_intent",
    "staged_low_value_rollout",
];

/// The lifecycle state of one strategy package (FR-STR-003).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyState {
    /// Initial state at registration; no execution evidence yet.
    Draft,
    /// golden + holdout + cost checks passed.
    Validated,
    /// parity + minimum observation window passed; Paper-trading live.
    Paper,
    /// Phase 3 safety evidence passed; eligible for Owner-only live rollout.
    LiveCandidate,
    /// Owner-retired; terminal state.
    Retired,
}

impl std::fmt::Display for StrategyState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Draft => "Draft",
            Self::Validated => "Validated",
            Self::Paper => "Paper",
            Self::LiveCandidate => "LiveCandidate",
            Self::Retired => "Retired",
        };
        f.write_str(name)
    }
}

/// A supported market.  The MVP supports exactly the Korean exchange (KRX);
/// anything else is a typed [`RegistryError::UnsupportedMarket`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Market {
    Krx,
}

impl Market {
    /// The single supported market, as a stable wire value.
    pub const SUPPORTED: &'static [&'static str] = &["krx"];

    /// Parses a market wire value; unsupported markets are typed errors.
    pub fn parse(value: &str) -> Result<Self, RegistryError> {
        match value {
            "krx" => Ok(Self::Krx),
            other => Err(RegistryError::UnsupportedMarket {
                market: other.to_owned(),
                supported: Market::SUPPORTED.join(","),
            }),
        }
    }

    /// The stable wire value.
    pub const fn as_str(&self) -> &'static str {
        "krx"
    }
}

/// A supported trading cadence.  The MVP supports exactly daily (EOD)
/// rebalancing; intraday cadences are typed errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Cadence {
    Daily,
}

impl Cadence {
    /// The single supported cadence.
    pub const SUPPORTED: &'static [&'static str] = &["daily"];

    /// Parses a cadence wire value; unsupported cadences are typed errors.
    pub fn parse(value: &str) -> Result<Self, RegistryError> {
        match value {
            "daily" => Ok(Self::Daily),
            other => Err(RegistryError::UnsupportedCadence {
                cadence: other.to_owned(),
                supported: Cadence::SUPPORTED.join(","),
            }),
        }
    }

    /// The stable wire value.
    pub const fn as_str(&self) -> &'static str {
        "daily"
    }
}

/// A supported asset class.  The MVP supports exactly ETFs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AssetClass {
    Etf,
}

impl AssetClass {
    /// The single supported asset class.
    pub const SUPPORTED: &'static [&'static str] = &["etf"];

    /// Parses an asset-class wire value.
    pub fn parse(value: &str) -> Result<Self, RegistryError> {
        match value {
            "etf" => Ok(Self::Etf),
            other => Err(RegistryError::UnsupportedAssetClass {
                asset_class: other.to_owned(),
                supported: AssetClass::SUPPORTED.join(","),
            }),
        }
    }

    /// The stable wire value.
    pub const fn as_str(&self) -> &'static str {
        "etf"
    }
}

/// The promotion-gate evidence required to reach a target state.  The
/// variant MUST match the target state's gate (see module docs); a wrong
/// variant is a typed [`RegistryError::MissingPromotionEvidence`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromotionEvidence {
    /// Validated gate: golden + holdout + cost checks, each a non-empty
    /// manifest hash produced by the reproducibility harness (Todo 6).
    Golden {
        golden_manifest_hash: String,
        holdout_manifest_hash: String,
        cost_manifest_hash: String,
    },
    /// Paper gate: a parity report (backtest/Paper ledger parity, Todo 30)
    /// plus a minimum observation window of live Paper sessions.
    Paper {
        parity_report_id: String,
        observation_sessions: u64,
    },
    /// LiveCandidate gate: a Phase 3 safety bundle covering every documented
    /// check in [`PHASE3_SAFETY_CHECKS`].
    Phase3 {
        safety_bundle_id: String,
        checks: BTreeSet<String>,
    },
}

/// The authenticated actor driving a registry operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actor {
    /// The station owner: may register, promote, retire, and deploy code.
    Owner,
    /// An invited member: may only store schema-bound parameter configs.
    Member(String),
}

impl Actor {
    /// The stable audit label of the actor.
    pub fn label(&self) -> String {
        match self {
            Self::Owner => "owner".to_owned(),
            Self::Member(user) => format!("member:{user}"),
        }
    }

    /// Whether the actor is the Owner.
    pub fn is_owner(&self) -> bool {
        matches!(self, Self::Owner)
    }
}

/// One immutable strategy package definition (FR-STR-001/004).  `state` and
/// `canonical_hash` are registry-managed: packages enter in `Draft`, and the
/// content hash covers the definition only (never the mutable state).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrategyPackage {
    /// The stable strategy id (e.g. `buy_and_hold`).
    pub strategy_id: String,
    /// The immutable semantic version of this definition.
    pub version: StrategyVersion,
    /// Human-readable name.
    pub name: String,
    /// What the strategy does.
    pub description: String,
    /// Documented risk characteristics (shown to Members).
    pub risk_description: String,
    /// The parameters JSON Schema (FR-STR-002: Member changes are validated
    /// against this schema, never code).
    pub parameter_schema: Json,
    /// The validated default parameters of this version.
    pub default_parameters: Json,
    /// Supported markets (exactly KRX for the MVP).
    pub markets: Vec<Market>,
    /// Supported asset classes (exactly ETF for the MVP).
    pub asset_classes: Vec<AssetClass>,
    /// Supported rebalance cadences (exactly daily for the MVP).
    pub cadences: Vec<Cadence>,
    /// Factor ids the target generator consumes (factor-engine, Todo 15).
    pub required_factors: BTreeSet<String>,
    /// Minimum populated sessions before the strategy can produce a target.
    pub minimum_lookback_sessions: u64,
    /// Reference to the engine-independent target generator.
    pub target_generator_ref: String,
    /// Reference to the NautilusTrader execution adapter (Todo 13 events).
    pub nt_adapter_ref: String,
    /// Golden fixture paths (deterministic expected targets).
    pub golden_fixture_refs: Vec<String>,
    /// The current lifecycle state (registry-managed).
    pub state: StrategyState,
    /// Immutable SHA-256 over the canonical definition bytes.
    pub canonical_hash: String,
}

impl StrategyPackage {
    /// The canonical bytes the content hash covers (the definition minus the
    /// mutable state and the hash itself).
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, RegistryError> {
        #[derive(Serialize)]
        struct Canonical<'a> {
            strategy_id: &'a str,
            version: &'a str,
            name: &'a str,
            description: &'a str,
            risk_description: &'a str,
            parameter_schema: &'a Json,
            default_parameters: &'a Json,
            markets: &'a [Market],
            asset_classes: &'a [AssetClass],
            cadences: &'a [Cadence],
            required_factors: &'a BTreeSet<String>,
            minimum_lookback_sessions: u64,
            target_generator_ref: &'a str,
            nt_adapter_ref: &'a str,
            golden_fixture_refs: &'a [String],
        }
        let canonical = Canonical {
            strategy_id: &self.strategy_id,
            version: &self.version.to_string(),
            name: &self.name,
            description: &self.description,
            risk_description: &self.risk_description,
            parameter_schema: &self.parameter_schema,
            default_parameters: &self.default_parameters,
            markets: &self.markets,
            asset_classes: &self.asset_classes,
            cadences: &self.cadences,
            required_factors: &self.required_factors,
            minimum_lookback_sessions: self.minimum_lookback_sessions,
            target_generator_ref: &self.target_generator_ref,
            nt_adapter_ref: &self.nt_adapter_ref,
            golden_fixture_refs: &self.golden_fixture_refs,
        };
        serde_json::to_vec(&canonical).map_err(|e| RegistryError::Internal {
            detail: format!("canonical package serialization failed: {e}"),
        })
    }

    /// The SHA-256 content hash: identical definitions hash identically.
    pub fn compute_canonical_hash(&self) -> Result<String, RegistryError> {
        Ok(ContentHash::from_bytes(&self.canonical_bytes()?).to_string())
    }

    /// Deterministic version ordering key (release > pre-release).
    pub(crate) fn version_sort_key(&self) -> (u64, u64, u64, bool, String) {
        let semver = self.version.inner();
        (
            semver.major(),
            semver.minor(),
            semver.patch(),
            semver.pre_release().is_none(),
            semver.pre_release().unwrap_or("").to_owned(),
        )
    }
}

/// A Member's schema-bound parameter config of one immutable version
/// (FR-STR-002).  The config never mutates the package.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemberStrategyConfig {
    pub config_id: String,
    pub actor: String,
    pub strategy_id: String,
    pub strategy_version: StrategyVersion,
    /// Validated against the package's JSON Schema.
    pub parameters: Json,
    pub seq: u64,
}

/// An Owner code deployment record (strategy code deployment is Owner-only).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeDeployment {
    pub deployment_id: String,
    pub actor: String,
    pub code_hash: String,
    pub seq: u64,
}

/// The outcome of an audited operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum AuditOutcome {
    Approved,
    Denied,
}

/// One append-only audit entry (design §7.3: audit rows are never mutated).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegistryAuditEntry {
    pub seq: u64,
    /// Milliseconds since the epoch (monotonic enough for ordering).
    pub at: String,
    pub actor: String,
    pub action: String,
    pub strategy_id: Option<String>,
    pub version: Option<String>,
    pub from_state: Option<StrategyState>,
    pub to_state: Option<StrategyState>,
    pub outcome: AuditOutcome,
    pub reason: String,
}

/// A successful promotion (or retirement).
#[derive(Debug, Clone, PartialEq)]
pub struct PromotionRecord {
    pub strategy_id: String,
    pub version: StrategyVersion,
    pub from: StrategyState,
    pub to: StrategyState,
    pub seq: u64,
}

/// A typed registry failure.  Every denial carries a stable machine-readable
/// code via [`RegistryError::code`].
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RegistryError {
    #[error("strategy {strategy_id} is not registered")]
    UnknownStrategy { strategy_id: String },
    #[error("strategy {strategy_id} has no version {version}")]
    UnknownVersion {
        strategy_id: String,
        version: String,
    },
    #[error("strategy {strategy_id} version {version} is immutable and already registered")]
    ImmutableVersion {
        strategy_id: String,
        version: String,
    },
    #[error("invalid strategy package: {detail}")]
    InvalidPackage { detail: String },
    #[error("parameters violate the package schema: {detail}")]
    InvalidParameters { detail: String },
    #[error("unsupported market {market} (MVP supports {supported})")]
    UnsupportedMarket { market: String, supported: String },
    #[error("unsupported cadence {cadence} (MVP supports {supported})")]
    UnsupportedCadence { cadence: String, supported: String },
    #[error("unsupported asset class {asset_class} (MVP supports {supported})")]
    UnsupportedAssetClass {
        asset_class: String,
        supported: String,
    },
    #[error("action {action} requires Owner; actor {actor} is not Owner")]
    Unauthorized { actor: String, action: String },
    #[error("promotion to {to_state} requires missing evidence: {missing}")]
    MissingPromotionEvidence { to_state: String, missing: String },
    #[error("invalid promotion {from} -> {to}: {detail}")]
    InvalidPromotion {
        from: StrategyState,
        to: StrategyState,
        detail: String,
    },
    #[error("member code upload denied: {detail}")]
    MemberCodeDenied { detail: String },
    #[error("internal registry error: {detail}")]
    Internal { detail: String },
}

impl RegistryError {
    /// The stable machine-readable code of the failure.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::UnknownStrategy { .. } => "UNKNOWN_STRATEGY",
            Self::UnknownVersion { .. } => "UNKNOWN_VERSION",
            Self::ImmutableVersion { .. } => "IMMUTABLE_VERSION",
            Self::InvalidPackage { .. } => "INVALID_PACKAGE",
            Self::InvalidParameters { .. } => "INVALID_PARAMETERS",
            Self::UnsupportedMarket { .. } => "UNSUPPORTED_MARKET",
            Self::UnsupportedCadence { .. } => "UNSUPPORTED_CADENCE",
            Self::UnsupportedAssetClass { .. } => "UNSUPPORTED_ASSET_CLASS",
            Self::Unauthorized { .. } => "UNAUTHORIZED",
            Self::MissingPromotionEvidence { .. } => "MISSING_PROMOTION_EVIDENCE",
            Self::InvalidPromotion { .. } => "INVALID_PROMOTION",
            Self::MemberCodeDenied { .. } => "MEMBER_CODE_DENIED",
            Self::Internal { .. } => "INTERNAL",
        }
    }
}

/// The promotion gate attached to one state transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromotionGate {
    Validated,
    Paper,
    LiveCandidate,
    Retire,
}

impl PromotionGate {
    /// The gate for a transition, enforcing the documented path (no skips,
    /// `Draft` is the entry state, `Retired` is terminal).
    fn for_transition(from: StrategyState, to: StrategyState) -> Result<Self, RegistryError> {
        let invalid = |detail: &str| RegistryError::InvalidPromotion {
            from,
            to,
            detail: detail.to_owned(),
        };
        match (from, to) {
            (_, StrategyState::Draft) => Err(invalid(
                "Draft is the entry state; promotion into Draft is not a transition",
            )),
            (StrategyState::Retired, _) => Err(invalid("Retired is terminal")),
            (_, StrategyState::Retired) => Ok(Self::Retire),
            (StrategyState::Draft, StrategyState::Validated) => Ok(Self::Validated),
            (StrategyState::Validated, StrategyState::Paper) => Ok(Self::Paper),
            (StrategyState::Paper, StrategyState::LiveCandidate) => Ok(Self::LiveCandidate),
            (StrategyState::Draft, StrategyState::Paper) => Err(invalid(
                "promotion must pass through Validated (golden+holdout+cost checks)",
            )),
            (StrategyState::Validated, StrategyState::LiveCandidate) => Err(invalid(
                "promotion must pass through Paper (parity + minimum observation window)",
            )),
            (StrategyState::Draft, StrategyState::LiveCandidate) => {
                Err(invalid("promotion must pass through Validated then Paper"))
            }
            (a, b) if a == b => Err(invalid(&format!("already {a}"))),
            _ => Err(invalid("unsupported transition")),
        }
    }

    /// Checks the submitted evidence against the gate (typed denials).
    fn require(&self, evidence: &PromotionEvidence) -> Result<(), RegistryError> {
        match self {
            Self::Retire => Ok(()),
            Self::Validated => {
                let PromotionEvidence::Golden {
                    golden_manifest_hash,
                    holdout_manifest_hash,
                    cost_manifest_hash,
                } = evidence
                else {
                    return Err(RegistryError::MissingPromotionEvidence {
                        to_state: "Validated".to_owned(),
                        missing: "golden".to_owned(),
                    });
                };
                let mut missing = Vec::new();
                if golden_manifest_hash.is_empty() {
                    missing.push("golden");
                }
                if holdout_manifest_hash.is_empty() {
                    missing.push("holdout");
                }
                if cost_manifest_hash.is_empty() {
                    missing.push("cost");
                }
                if missing.is_empty() {
                    Ok(())
                } else {
                    Err(RegistryError::MissingPromotionEvidence {
                        to_state: "Validated".to_owned(),
                        missing: missing.join(","),
                    })
                }
            }
            Self::Paper => {
                let PromotionEvidence::Paper {
                    parity_report_id,
                    observation_sessions,
                } = evidence
                else {
                    return Err(RegistryError::MissingPromotionEvidence {
                        to_state: "Paper".to_owned(),
                        missing: "parity_report".to_owned(),
                    });
                };
                if parity_report_id.is_empty() {
                    return Err(RegistryError::MissingPromotionEvidence {
                        to_state: "Paper".to_owned(),
                        missing: "parity_report".to_owned(),
                    });
                }
                if *observation_sessions < MIN_PAPER_OBSERVATION_SESSIONS {
                    return Err(RegistryError::InvalidPromotion {
                        from: StrategyState::Validated,
                        to: StrategyState::Paper,
                        detail: format!(
                            "observation window {observation_sessions} sessions is below the \
                             minimum {MIN_PAPER_OBSERVATION_SESSIONS}"
                        ),
                    });
                }
                Ok(())
            }
            Self::LiveCandidate => {
                let PromotionEvidence::Phase3 { checks, .. } = evidence else {
                    return Err(RegistryError::MissingPromotionEvidence {
                        to_state: "LiveCandidate".to_owned(),
                        missing: "phase3_safety".to_owned(),
                    });
                };
                let missing: Vec<&str> = PHASE3_SAFETY_CHECKS
                    .iter()
                    .filter(|check| !checks.contains(**check))
                    .copied()
                    .collect();
                if missing.is_empty() {
                    Ok(())
                } else {
                    Err(RegistryError::MissingPromotionEvidence {
                        to_state: "LiveCandidate".to_owned(),
                        missing: missing.join(","),
                    })
                }
            }
        }
    }
}

/// The strategy registry: versioned immutable packages + state machine +
/// append-only audit + the Member schema-config boundary.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    /// strategy_id -> versions in registration order (id-keyed for
    /// deterministic iteration).
    packages: BTreeMap<String, Vec<StrategyPackage>>,
    audit: Vec<RegistryAuditEntry>,
    configs: Vec<MemberStrategyConfig>,
    deployments: Vec<CodeDeployment>,
    seq: u64,
}

impl Registry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers an immutable package version (Owner-only).  The package must
    /// be in `Draft`, its definition valid, and its defaults schema-valid.
    /// Re-registering an existing (id, version) is an immutability denial.
    pub fn register(
        &mut self,
        actor: &Actor,
        package: StrategyPackage,
    ) -> Result<StrategyPackage, RegistryError> {
        if !actor.is_owner() {
            self.audit_denied(AuditSpec {
                actor,
                action: "REGISTER",
                strategy_id: Some(&package.strategy_id),
                version: Some(&package.version.to_string()),
                from: None,
                to: None,
                reason: "strategy registration is Owner-only".to_owned(),
            });
            return Err(RegistryError::Unauthorized {
                actor: actor.label(),
                action: "REGISTER".to_owned(),
            });
        }
        if let Err(error) = validate_package(&package) {
            self.audit_denied(AuditSpec {
                actor,
                action: "REGISTER",
                strategy_id: Some(&package.strategy_id),
                version: Some(&package.version.to_string()),
                from: None,
                to: None,
                reason: error.to_string(),
            });
            return Err(error);
        }
        let exists = self
            .packages
            .get(&package.strategy_id)
            .is_some_and(|versions| versions.iter().any(|p| p.version == package.version));
        if exists {
            let error = RegistryError::ImmutableVersion {
                strategy_id: package.strategy_id.clone(),
                version: package.version.to_string(),
            };
            self.audit_denied(AuditSpec {
                actor,
                action: "REGISTER",
                strategy_id: Some(&package.strategy_id),
                version: Some(&package.version.to_string()),
                from: None,
                to: None,
                reason: error.to_string(),
            });
            return Err(error);
        }
        let hash = package.compute_canonical_hash()?;
        let stored = StrategyPackage {
            canonical_hash: hash,
            ..package
        };
        self.packages
            .entry(stored.strategy_id.clone())
            .or_default()
            .push(stored.clone());
        self.audit_approved(AuditSpec {
            actor,
            action: "REGISTER",
            strategy_id: Some(&stored.strategy_id),
            version: Some(&stored.version.to_string()),
            from: None,
            to: None,
            reason: "package version registered".to_owned(),
        });
        Ok(stored)
    }

    /// Resolves the ORIGINAL immutable definition of an exact version
    /// (FR-STR-001: old runs keep their version after a new release).
    pub fn resolve(
        &self,
        strategy_id: &str,
        version: &str,
    ) -> Result<&StrategyPackage, RegistryError> {
        let parsed =
            StrategyVersion::parse(version).map_err(|_| RegistryError::UnknownVersion {
                strategy_id: strategy_id.to_owned(),
                version: version.to_owned(),
            })?;
        let versions =
            self.packages
                .get(strategy_id)
                .ok_or_else(|| RegistryError::UnknownStrategy {
                    strategy_id: strategy_id.to_owned(),
                })?;
        versions
            .iter()
            .find(|p| p.version == parsed)
            .ok_or_else(|| RegistryError::UnknownVersion {
                strategy_id: strategy_id.to_owned(),
                version: version.to_owned(),
            })
    }

    /// Resolves the highest registered version of a strategy (the newest
    /// definition, even if it is Retired).
    pub fn resolve_latest(&self, strategy_id: &str) -> Result<&StrategyPackage, RegistryError> {
        let versions =
            self.packages
                .get(strategy_id)
                .ok_or_else(|| RegistryError::UnknownStrategy {
                    strategy_id: strategy_id.to_owned(),
                })?;
        versions
            .iter()
            .max_by_key(|p| p.version_sort_key())
            .ok_or_else(|| RegistryError::UnknownVersion {
                strategy_id: strategy_id.to_owned(),
                version: "latest".to_owned(),
            })
    }

    /// All registered packages (id-keyed, deterministic order).
    pub fn all_packages(&self) -> Vec<&StrategyPackage> {
        self.packages.values().flatten().collect()
    }

    /// Promotes (or retires) a package version through the documented gates
    /// (Owner-only; every outcome is audited).
    pub fn promote(
        &mut self,
        actor: &Actor,
        strategy_id: &str,
        version: &str,
        to: StrategyState,
        evidence: PromotionEvidence,
    ) -> Result<PromotionRecord, RegistryError> {
        let current = self.resolve(strategy_id, version).cloned()?;
        if !actor.is_owner() {
            let error = RegistryError::Unauthorized {
                actor: actor.label(),
                action: "PROMOTE".to_owned(),
            };
            self.audit_denied(AuditSpec {
                actor,
                action: "PROMOTE",
                strategy_id: Some(strategy_id),
                version: Some(version),
                from: Some(current.state),
                to: Some(to),
                reason: error.to_string(),
            });
            return Err(error);
        }
        let gate = PromotionGate::for_transition(current.state, to)?;
        if let Err(error) = gate.require(&evidence) {
            self.audit_denied(AuditSpec {
                actor,
                action: "PROMOTE",
                strategy_id: Some(strategy_id),
                version: Some(version),
                from: Some(current.state),
                to: Some(to),
                reason: error.to_string(),
            });
            return Err(error);
        }
        let slot = self
            .packages
            .get_mut(strategy_id)
            .and_then(|versions| versions.iter_mut().find(|p| p.version == current.version))
            .expect("resolved version must exist");
        slot.state = to;
        let record = PromotionRecord {
            strategy_id: strategy_id.to_owned(),
            version: current.version,
            from: current.state,
            to,
            seq: self.seq,
        };
        self.audit_approved(AuditSpec {
            actor,
            action: "PROMOTE",
            strategy_id: Some(strategy_id),
            version: Some(version),
            from: Some(record.from),
            to: Some(to),
            reason: format!("promoted to {to}"),
        });
        Ok(record)
    }

    /// Retires a package version (Owner-only; terminal).
    pub fn retire(
        &mut self,
        actor: &Actor,
        strategy_id: &str,
        version: &str,
    ) -> Result<PromotionRecord, RegistryError> {
        self.promote(
            actor,
            strategy_id,
            version,
            StrategyState::Retired,
            PromotionEvidence::Phase3 {
                safety_bundle_id: String::new(),
                checks: BTreeSet::new(),
            },
        )
    }

    /// Stores a Member's schema-bound parameter config of one immutable
    /// version (FR-STR-002).  Parameters are validated against the package's
    /// JSON Schema; the package itself is never mutated.
    pub fn apply_member_config(
        &mut self,
        actor: &Actor,
        strategy_id: &str,
        version: &str,
        parameters: Json,
    ) -> Result<MemberStrategyConfig, RegistryError> {
        let package = self.resolve(strategy_id, version).cloned()?;
        if let Err(error) = validate_against_schema(&package.parameter_schema, &parameters) {
            self.audit_denied(AuditSpec {
                actor,
                action: "MEMBER_CONFIG",
                strategy_id: Some(strategy_id),
                version: Some(version),
                from: None,
                to: None,
                reason: error.to_string(),
            });
            return Err(error);
        }
        let config = MemberStrategyConfig {
            config_id: format!("member-config-{}", self.configs.len() + 1),
            actor: actor.label(),
            strategy_id: strategy_id.to_owned(),
            strategy_version: package.version,
            parameters,
            seq: self.seq,
        };
        self.audit_approved(AuditSpec {
            actor,
            action: "MEMBER_CONFIG",
            strategy_id: Some(strategy_id),
            version: Some(version),
            from: None,
            to: None,
            reason: "schema-valid member parameter config stored".to_owned(),
        });
        self.configs.push(config.clone());
        Ok(config)
    }

    /// Strategy code deployment: Owner-only.  Any Member code upload is a
    /// typed, audited [`RegistryError::MemberCodeDenied`] — Member changes
    /// are schema-bound parameter configs only.
    pub fn deploy_code(
        &mut self,
        actor: &Actor,
        code: &str,
    ) -> Result<CodeDeployment, RegistryError> {
        match actor {
            Actor::Owner => {
                let deployment = CodeDeployment {
                    deployment_id: format!("deployment-{}", self.deployments.len() + 1),
                    actor: actor.label(),
                    code_hash: ContentHash::from_bytes(code.as_bytes()).to_string(),
                    seq: self.seq,
                };
                self.audit_approved(AuditSpec {
                    actor,
                    action: "DEPLOY_CODE",
                    strategy_id: None,
                    version: None,
                    from: None,
                    to: None,
                    reason: "Owner code deployment recorded".to_owned(),
                });
                self.deployments.push(deployment.clone());
                Ok(deployment)
            }
            Actor::Member(_) => {
                let detail = "strategy code deployment is Owner-only; Member changes are \
                              schema-bound parameter configs"
                    .to_owned();
                self.audit_denied(AuditSpec {
                    actor,
                    action: "DEPLOY_CODE",
                    strategy_id: None,
                    version: None,
                    from: None,
                    to: None,
                    reason: detail.clone(),
                });
                Err(RegistryError::MemberCodeDenied { detail })
            }
        }
    }

    /// The append-only audit log (never mutated after append).
    pub fn audit(&self) -> &[RegistryAuditEntry] {
        &self.audit
    }

    /// Stored Member parameter configs.
    pub fn configs(&self) -> &[MemberStrategyConfig] {
        &self.configs
    }

    /// Recorded Owner code deployments.
    pub fn deployments(&self) -> &[CodeDeployment] {
        &self.deployments
    }

    fn audit_approved(&mut self, spec: AuditSpec<'_>) {
        self.push_audit(spec, AuditOutcome::Approved);
    }

    fn audit_denied(&mut self, spec: AuditSpec<'_>) {
        self.push_audit(spec, AuditOutcome::Denied);
    }

    fn push_audit(&mut self, spec: AuditSpec<'_>, outcome: AuditOutcome) {
        self.seq += 1;
        self.audit.push(RegistryAuditEntry {
            seq: self.seq,
            at: now_millis(),
            actor: spec.actor.label(),
            action: spec.action.to_owned(),
            strategy_id: spec.strategy_id.map(str::to_owned),
            version: spec.version.map(str::to_owned),
            from_state: spec.from,
            to_state: spec.to,
            outcome,
            reason: spec.reason,
        });
    }
}

/// The parameters of one audited operation.
struct AuditSpec<'a> {
    actor: &'a Actor,
    action: &'a str,
    strategy_id: Option<&'a str>,
    version: Option<&'a str>,
    from: Option<StrategyState>,
    to: Option<StrategyState>,
    reason: String,
}

fn now_millis() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().to_string())
        .unwrap_or_else(|_| "0".to_owned())
}

/// Validates a package definition before registration.
fn validate_package(package: &StrategyPackage) -> Result<(), RegistryError> {
    let invalid = |detail: String| RegistryError::InvalidPackage { detail };
    let valid_id = !package.strategy_id.is_empty()
        && package
            .strategy_id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !valid_id {
        return Err(invalid(format!(
            "strategy_id {:?} must match ^[a-z0-9_]+$",
            package.strategy_id
        )));
    }
    if package.name.is_empty()
        || package.description.is_empty()
        || package.risk_description.is_empty()
    {
        return Err(invalid(
            "name, description, and risk_description must not be empty".to_owned(),
        ));
    }
    if package.state != StrategyState::Draft {
        return Err(invalid(format!(
            "packages enter the registry in Draft (got {})",
            package.state
        )));
    }
    if package.markets.is_empty() {
        return Err(invalid("markets must not be empty".to_owned()));
    }
    if package.asset_classes.is_empty() {
        return Err(invalid("asset_classes must not be empty".to_owned()));
    }
    if package.cadences.is_empty() {
        return Err(invalid("cadences must not be empty".to_owned()));
    }
    if package.target_generator_ref.is_empty() || package.nt_adapter_ref.is_empty() {
        return Err(invalid(
            "target_generator_ref and nt_adapter_ref must not be empty".to_owned(),
        ));
    }
    for factor in &package.required_factors {
        if factor.is_empty() {
            return Err(invalid("required factor ids must not be empty".to_owned()));
        }
    }
    if package.required_factors.is_empty() && package.minimum_lookback_sessions != 0 {
        return Err(invalid(
            "an empty required_factors set requires minimum_lookback_sessions == 0".to_owned(),
        ));
    }
    validate_schema_shape(&package.parameter_schema)?;
    validate_against_schema(&package.parameter_schema, &package.default_parameters).map_err(
        |error| match error {
            RegistryError::InvalidParameters { detail } => RegistryError::InvalidPackage {
                detail: format!("default_parameters violate the package schema: {detail}"),
            },
            other => other,
        },
    )
}

/// The schema document itself must be a valid object schema.
fn validate_schema_shape(schema: &Json) -> Result<(), RegistryError> {
    let invalid = |detail: String| RegistryError::InvalidPackage { detail };
    let object = schema
        .as_object()
        .ok_or_else(|| invalid("parameter_schema must be a JSON object".to_owned()))?;
    if object.get("type").and_then(Json::as_str) != Some("object") {
        return Err(invalid(
            "parameter_schema.type must be \"object\"".to_owned(),
        ));
    }
    if !object.get("properties").and_then(Json::as_object).is_some() {
        return Err(invalid(
            "parameter_schema.properties must be an object".to_owned(),
        ));
    }
    jsonschema::Validator::new(schema).map_err(|error| {
        invalid(format!(
            "parameter_schema is not valid JSON Schema: {error}"
        ))
    })?;
    Ok(())
}

/// Validates an instance (defaults or Member parameters) against a schema.
fn validate_against_schema(schema: &Json, instance: &Json) -> Result<(), RegistryError> {
    let validator =
        jsonschema::Validator::new(schema).map_err(|error| RegistryError::InvalidParameters {
            detail: format!("schema is not a valid JSON Schema: {error}"),
        })?;
    validator
        .validate(instance)
        .map_err(|error| RegistryError::InvalidParameters {
            detail: format!("parameters violate the package schema: {error}"),
        })
}
