//! Fail-closed validation of the untrusted target-generator result.

use std::collections::{BTreeMap, BTreeSet};

use domain::ContentHash;
use serde_json::{Number, Value, json};
use thiserror::Error;

use crate::recommendation::child::{Reason, TargetChildOutput, TargetProvenance};
use crate::recommendation::compute::AttestedUniverse;
use crate::recommendation::input::AttestedDataset;
use crate::types::ErrorClass;

const DB_SCALE: u32 = 6;
const DB_SCALE_USIZE: usize = 6;
const SCALE_FACTOR: f64 = 1_000_000.0;
const MAX_TOLERANCE: f64 = 0.000_001;

#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedItem {
    pub(super) instrument_id: String,
    pub(super) rank: Option<i32>,
    pub(super) target_weight: Option<String>,
    pub(super) reason_codes: Value,
    pub(super) factors_json: Value,
    pub(super) excluded: bool,
    pub(super) exclusion_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedPortfolio {
    pub(super) items: Vec<ValidatedItem>,
    pub(super) positive_weights: BTreeMap<String, String>,
    pub(super) cash_weight: String,
    pub(super) selected_count: usize,
    pub(super) excluded_count: usize,
    pub(super) portfolio_snapshot_id: String,
    pub(super) portfolio_reasons: Value,
    pub(super) universe_snapshot_id: String,
    pub(super) factor_snapshot_hash: String,
}

impl ValidatedItem {
    pub fn excluded(&self) -> bool {
        self.excluded
    }

    pub fn reason_codes(&self) -> &Value {
        &self.reason_codes
    }
}

impl ValidatedPortfolio {
    pub fn items(&self) -> &[ValidatedItem] {
        &self.items
    }

    pub fn cash_weight(&self) -> &str {
        &self.cash_weight
    }

    pub fn selected_count(&self) -> usize {
        self.selected_count
    }

    pub fn excluded_count(&self) -> usize {
        self.excluded_count
    }

    pub fn portfolio_snapshot_id(&self) -> &str {
        &self.portfolio_snapshot_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RecommendationValidationError {
    #[error("invalid recommendation result: {detail}")]
    Input { detail: String },
    #[error("recommendation result integrity failure: {detail}")]
    Integrity { detail: String },
    #[error("recommendation result determinism failure: {detail}")]
    Determinism { detail: String },
    #[error("recommendation portfolio hash mismatch: {detail}")]
    HashMismatch { detail: String },
}

impl RecommendationValidationError {
    pub const fn class(&self) -> ErrorClass {
        match self {
            Self::Input { .. } => ErrorClass::Input,
            Self::Integrity { .. } => ErrorClass::Integrity,
            Self::Determinism { .. } | Self::HashMismatch { .. } => ErrorClass::Determinism,
        }
    }

    pub const fn code(&self) -> &'static str {
        match self {
            Self::Input { .. } => "RECOMMENDATION_RESULT_INVALID",
            Self::Integrity { .. } => "RECOMMENDATION_RESULT_INTEGRITY",
            Self::Determinism { .. } => "RECOMMENDATION_RESULT_DETERMINISM",
            Self::HashMismatch { .. } => "RECOMMENDATION_PORTFOLIO_HASH_MISMATCH",
        }
    }
}

/// Validate every identity, membership and economic invariant before any row
/// is made visible. `expected_provenance` must itself agree with the attested
/// database dataset and the shipped universe; a caller cannot weaken the
/// checks by constructing a different expected object.
pub fn validate_target_output(
    mut output: TargetChildOutput,
    expected_strategy_id: &str,
    expected_strategy_version: &str,
    expected_as_of: &str,
    universe: &AttestedUniverse,
    dataset: &AttestedDataset,
    expected_provenance: &TargetProvenance,
) -> Result<ValidatedPortfolio, RecommendationValidationError> {
    validate_expected_provenance(universe, dataset, expected_provenance)?;
    require_integrity(
        output.strategy_version == format!("{expected_strategy_id}@{expected_strategy_version}"),
        "strategy id or version does not match",
    )?;
    require_integrity(output.as_of == expected_as_of, "as-of does not match")?;
    require_integrity(
        output.dataset_version_id == expected_provenance.dataset_version_id
            && output.dataset_id == expected_provenance.dataset_id
            && output.dataset_version == expected_provenance.dataset_version
            && output.curated_version == expected_provenance.curated_version
            && output.dataset_manifest_sha256 == expected_provenance.dataset_manifest_sha256
            && output.universe_snapshot_id == expected_provenance.universe_snapshot_id
            && output.factor_snapshot_hash == expected_provenance.factor_snapshot_hash,
        "result provenance does not match attested input",
    )?;

    validate_constraints(&output)?;
    validate_reason_list(&output.portfolio_reasons, "portfolio")?;
    require_integrity(
        !output.targets.is_empty() || !output.portfolio_reasons.is_empty(),
        "all-cash output must carry an explicit portfolio reason",
    )?;

    let expected_members: BTreeSet<&str> = universe.members.iter().map(String::as_str).collect();
    let mut observed = BTreeSet::new();
    let mut ranks = BTreeSet::new();
    let mut items = Vec::with_capacity(universe.members.len());
    let mut positive_weights = BTreeMap::new();
    let mut quantized_sum = 0_i64;

    for target in &output.targets {
        require_integrity(
            expected_members.contains(target.instrument_id.as_str()),
            "target contains a foreign instrument",
        )?;
        require_integrity(
            observed.insert(target.instrument_id.as_str()),
            "instrument occurs more than once across targets and exclusions",
        )?;
        require_integrity(target.rank > 0, "target rank must be positive")?;
        require_integrity(ranks.insert(target.rank), "target rank is duplicated")?;
        require_integrity(target.score.is_finite(), "target score must be finite")?;
        require_integrity(
            target.target_weight.is_finite()
                && target.target_weight > 0.0
                && target.target_weight <= 1.0,
            "selected target weight must be finite, positive, and at most one",
        )?;
        require_integrity(
            target.target_weight <= output.constraints.max_weight + output.constraints.tolerance,
            "target weight exceeds the declared maximum",
        )?;
        validate_factor_map(&target.factors)?;
        validate_reason_list(&target.reasons, "target")?;
        require_integrity(
            !target.reasons.is_empty(),
            "selected target must have a reason",
        )?;

        let (weight, scaled) = quantize(target.target_weight, output.constraints.tolerance)?;
        quantized_sum = quantized_sum
            .checked_add(scaled)
            .ok_or_else(|| integrity("quantized target sum overflowed"))?;
        if scaled > 0 {
            positive_weights.insert(target.instrument_id.clone(), weight.clone());
        }
        let rank = i32::try_from(target.rank)
            .map_err(|_| integrity("target rank exceeds database range"))?;
        items.push(ValidatedItem {
            instrument_id: target.instrument_id.clone(),
            rank: Some(rank),
            target_weight: Some(weight),
            reason_codes: reason_codes(&target.reasons),
            factors_json: serde_json::to_value(&target.factors)
                .map_err(|_| integrity("target factors cannot be represented as JSON"))?,
            excluded: false,
            exclusion_reason: None,
        });
    }

    let expected_ranks: BTreeSet<usize> = (1..=output.targets.len()).collect();
    require_integrity(ranks == expected_ranks, "target ranks must be contiguous")?;

    for exclusion in &output.exclusions {
        require_integrity(
            expected_members.contains(exclusion.instrument_id.as_str()),
            "exclusions contain a foreign instrument",
        )?;
        require_integrity(
            observed.insert(exclusion.instrument_id.as_str()),
            "instrument occurs more than once across targets and exclusions",
        )?;
        validate_reason_list(&exclusion.reasons, "exclusion")?;
        require_integrity(
            !exclusion.reasons.is_empty(),
            "excluded instrument must have a reason",
        )?;
        items.push(ValidatedItem {
            instrument_id: exclusion.instrument_id.clone(),
            rank: None,
            target_weight: None,
            reason_codes: reason_codes(&exclusion.reasons),
            factors_json: json!({}),
            excluded: true,
            exclusion_reason: exclusion
                .reasons
                .first()
                .map(|reason| reason.text_en.clone()),
        });
    }

    // Generators are allowed to omit canonical members they did not select
    // (notably buy-and-hold). Publication is not: normalize each omission to
    // one explicit Rust-owned exclusion without changing the child bytes or
    // the child portfolio hash checked below.
    for instrument_id in &universe.members {
        if !observed.contains(instrument_id.as_str()) {
            items.push(ValidatedItem {
                instrument_id: instrument_id.clone(),
                rank: None,
                target_weight: None,
                reason_codes: json!(["NOT_SELECTED_BY_STRATEGY"]),
                factors_json: json!({}),
                excluded: true,
                exclusion_reason: Some(
                    "전략이 이 고정 유니버스 종목을 선택하지 않았습니다. / The strategy did not select this canonical universe member."
                        .into(),
                ),
            });
        }
    }

    let (cash_weight, cash_scaled) = quantize(output.cash_weight, output.constraints.tolerance)?;
    quantized_sum = quantized_sum
        .checked_add(cash_scaled)
        .ok_or_else(|| integrity("quantized portfolio sum overflowed"))?;
    require_integrity(
        quantized_sum == SCALE_FACTOR as i64,
        "six-place target weights and cash must sum exactly to one",
    )?;

    let recomputed = canonical_portfolio_snapshot_id(&output)?;
    if output.portfolio_snapshot_id != recomputed {
        return Err(RecommendationValidationError::HashMismatch {
            detail: "child portfolio snapshot hash does not match canonical content".into(),
        });
    }

    items.sort_by(|left, right| left.instrument_id.cmp(&right.instrument_id));
    let portfolio_reasons = serde_json::to_value(&output.portfolio_reasons)
        .map_err(|_| integrity("portfolio reasons cannot be represented as JSON"))?;
    let selected_count = positive_weights.len();
    let excluded_count = items.iter().filter(|item| item.excluded).count();
    output.portfolio_snapshot_id = recomputed.clone();
    Ok(ValidatedPortfolio {
        items,
        positive_weights,
        cash_weight,
        selected_count,
        excluded_count,
        portfolio_snapshot_id: recomputed,
        portfolio_reasons,
        universe_snapshot_id: output.universe_snapshot_id,
        factor_snapshot_hash: output.factor_snapshot_hash,
    })
}

fn validate_expected_provenance(
    universe: &AttestedUniverse,
    dataset: &AttestedDataset,
    provenance: &TargetProvenance,
) -> Result<(), RecommendationValidationError> {
    require_integrity(
        provenance.dataset_version_id == dataset.id
            && provenance.dataset_id == dataset.dataset_id
            && provenance.dataset_version == dataset.version
            && provenance.curated_version == dataset.curated_version
            && provenance.dataset_manifest_sha256 == dataset.manifest_sha256
            && provenance.universe_snapshot_id == universe.snapshot_id,
        "expected provenance is not derived from attested input",
    )?;
    require_integrity(
        canonical_sha256(&provenance.universe_snapshot_id)
            && canonical_sha256(&provenance.factor_snapshot_hash)
            && canonical_plain_sha256(&provenance.dataset_manifest_sha256),
        "expected provenance contains a malformed hash",
    )
}

fn validate_constraints(output: &TargetChildOutput) -> Result<(), RecommendationValidationError> {
    let constraints = &output.constraints;
    if constraints.top_n == 0
        || constraints.top_n > 11
        || output.targets.len() > constraints.top_n
        || constraints.weight_scale == 0
        || u32::from(constraints.weight_scale) > DB_SCALE
        || !constraints.tolerance.is_finite()
        || constraints.tolerance <= 0.0
        || constraints.tolerance > MAX_TOLERANCE
    {
        return Err(RecommendationValidationError::Input {
            detail: "constraint bounds are invalid".into(),
        });
    }
    require_integrity(
        constraints.max_weight.is_finite()
            && (0.0..=1.0).contains(&constraints.max_weight)
            && constraints.cash_floor.is_finite()
            && (0.0..=1.0).contains(&constraints.cash_floor)
            && output.cash_weight.is_finite()
            && (0.0..=1.0).contains(&output.cash_weight)
            && output.cash_weight + constraints.tolerance >= constraints.cash_floor,
        "cash or constraint weights are invalid",
    )?;
    let total = output.cash_weight
        + output
            .targets
            .iter()
            .map(|target| target.target_weight)
            .sum::<f64>();
    require_integrity(
        total.is_finite() && (total - 1.0).abs() <= constraints.tolerance,
        "target weights and cash do not sum to one",
    )
}

fn validate_factor_map(
    factors: &BTreeMap<String, f64>,
) -> Result<(), RecommendationValidationError> {
    for (factor_id, value) in factors {
        require_integrity(valid_identifier(factor_id, false), "factor id is invalid")?;
        require_integrity(value.is_finite(), "factor value must be finite")?;
    }
    Ok(())
}

fn validate_reason_list(
    reasons: &[Reason],
    label: &str,
) -> Result<(), RecommendationValidationError> {
    let mut codes = BTreeSet::new();
    for reason in reasons {
        require_integrity(
            valid_identifier(&reason.code, true),
            &format!("{label} reason code is invalid"),
        )?;
        require_integrity(
            codes.insert(reason.code.as_str()),
            &format!("{label} reason code is duplicated"),
        )?;
        require_integrity(
            nonempty_bounded(&reason.text_ko, 512) && nonempty_bounded(&reason.text_en, 512),
            &format!("{label} reason text is invalid"),
        )?;
        for (key, value) in &reason.params {
            require_integrity(
                valid_identifier(key, false) && nonempty_bounded(value, 256),
                &format!("{label} reason parameter is invalid"),
            )?;
        }
    }
    Ok(())
}

fn reason_codes(reasons: &[Reason]) -> Value {
    Value::Array(
        reasons
            .iter()
            .map(|reason| Value::String(reason.code.clone()))
            .collect(),
    )
}

fn valid_identifier(value: &str, uppercase: bool) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.bytes().all(|byte| {
            byte.is_ascii_digit()
                || byte == b'_'
                || if uppercase {
                    byte.is_ascii_uppercase()
                } else {
                    byte.is_ascii_lowercase()
                }
        })
}

fn nonempty_bounded(value: &str, max: usize) -> bool {
    !value.is_empty() && value.trim() == value && value.len() <= max
}

fn canonical_sha256(value: &str) -> bool {
    value.len() == 71 && value.starts_with("sha256:") && canonical_plain_sha256(&value[7..])
}

fn canonical_plain_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn quantize(value: f64, tolerance: f64) -> Result<(String, i64), RecommendationValidationError> {
    require_integrity(value.is_finite(), "weight must be finite")?;
    let scaled_float = value * SCALE_FACTOR;
    let scaled = scaled_float.round() as i64;
    let quantized = scaled as f64 / SCALE_FACTOR;
    require_determinism(
        (quantized - value).abs() <= tolerance,
        "six-place quantization would change portfolio economics",
    )?;
    Ok((format!("{quantized:.DB_SCALE_USIZE$}"), scaled))
}

/// Reproduce Python's `json.dumps(sort_keys=True, ensure_ascii=False)` bytes
/// over every output field except the hash itself.
pub fn canonical_portfolio_snapshot_id(
    output: &TargetChildOutput,
) -> Result<String, RecommendationValidationError> {
    let value = json!({
        "as_of": output.as_of,
        "strategy_version": output.strategy_version,
        "universe_snapshot_id": output.universe_snapshot_id,
        "factor_snapshot_hash": output.factor_snapshot_hash,
        "dataset_version_id": output.dataset_version_id,
        "dataset_id": output.dataset_id,
        "dataset_version": output.dataset_version,
        "curated_version": output.curated_version,
        "dataset_manifest_sha256": output.dataset_manifest_sha256,
        "targets": output.targets,
        "exclusions": output.exclusions,
        "cash_weight": output.cash_weight,
        "constraints": output.constraints,
        "portfolio_reasons": output.portfolio_reasons,
    });
    let mut bytes = Vec::new();
    write_python_json(&value, &mut bytes)?;
    Ok(ContentHash::from_bytes(&bytes).as_str().to_owned())
}

fn write_python_json(
    value: &Value,
    output: &mut Vec<u8>,
) -> Result<(), RecommendationValidationError> {
    match value {
        Value::Null => output.extend_from_slice(b"null"),
        Value::Bool(value) => output.extend_from_slice(if *value { b"true" } else { b"false" }),
        Value::Number(value) => output.extend_from_slice(python_number(value)?.as_bytes()),
        Value::String(value) => {
            let encoded = serde_json::to_string(value)
                .map_err(|_| determinism("string cannot be serialized canonically"))?;
            output.extend_from_slice(encoded.as_bytes());
        }
        Value::Array(values) => {
            output.push(b'[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.extend_from_slice(b", ");
                }
                write_python_json(value, output)?;
            }
            output.push(b']');
        }
        Value::Object(values) => {
            output.push(b'{');
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(right.0));
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index > 0 {
                    output.extend_from_slice(b", ");
                }
                let encoded = serde_json::to_string(key)
                    .map_err(|_| determinism("object key cannot be serialized canonically"))?;
                output.extend_from_slice(encoded.as_bytes());
                output.extend_from_slice(b": ");
                write_python_json(value, output)?;
            }
            output.push(b'}');
        }
    }
    Ok(())
}

fn python_number(number: &Number) -> Result<String, RecommendationValidationError> {
    if number.is_i64() || number.is_u64() {
        return Ok(number.to_string());
    }
    let value = number
        .as_f64()
        .filter(|value| value.is_finite())
        .ok_or_else(|| determinism("non-finite number has no canonical JSON form"))?;
    if value == 0.0 {
        return Ok(if value.is_sign_negative() {
            "-0.0"
        } else {
            "0.0"
        }
        .into());
    }
    let raw = number.to_string().to_ascii_lowercase();
    let negative = raw.starts_with('-');
    let unsigned = raw.trim_start_matches('-');
    let (mantissa, explicit_exponent) = unsigned
        .split_once('e')
        .map_or((unsigned, 0_i32), |(mantissa, exponent)| {
            (mantissa, exponent.parse::<i32>().unwrap_or(0))
        });
    let decimal_index = mantissa.find('.').unwrap_or(mantissa.len());
    let raw_digits = mantissa.replace('.', "");
    let first_nonzero = raw_digits
        .bytes()
        .position(|byte| byte != b'0')
        .ok_or_else(|| determinism("zero number was not canonicalized"))?;
    let mut digits = raw_digits[first_nonzero..].to_owned();
    let exponent = explicit_exponent + decimal_index as i32 - first_nonzero as i32 - 1;
    while digits.len() > 1 && digits.ends_with('0') {
        digits.pop();
    }
    let mut rendered = if !(-4..16).contains(&exponent) {
        let mut value = String::new();
        value.push(digits.as_bytes()[0] as char);
        if digits.len() > 1 {
            value.push('.');
            value.push_str(&digits[1..]);
        }
        value.push('e');
        value.push(if exponent >= 0 { '+' } else { '-' });
        value.push_str(&format!("{:02}", exponent.unsigned_abs()));
        value
    } else if exponent < 0 {
        format!("0.{}{}", "0".repeat((-exponent - 1) as usize), digits)
    } else {
        let whole_len = exponent as usize + 1;
        if digits.len() <= whole_len {
            format!("{}{}", digits, "0".repeat(whole_len - digits.len()))
        } else {
            format!("{}.{}", &digits[..whole_len], &digits[whole_len..])
        }
    };
    if !rendered.contains(['.', 'e']) {
        rendered.push_str(".0");
    }
    if negative {
        rendered.insert(0, '-');
    }
    Ok(rendered)
}

fn require_integrity(condition: bool, detail: &str) -> Result<(), RecommendationValidationError> {
    if condition {
        Ok(())
    } else {
        Err(integrity(detail))
    }
}

fn require_determinism(condition: bool, detail: &str) -> Result<(), RecommendationValidationError> {
    if condition {
        Ok(())
    } else {
        Err(determinism(detail))
    }
}

fn integrity(detail: &str) -> RecommendationValidationError {
    RecommendationValidationError::Integrity {
        detail: detail.to_owned(),
    }
}

fn determinism(detail: &str) -> RecommendationValidationError {
    RecommendationValidationError::Determinism {
        detail: detail.to_owned(),
    }
}
