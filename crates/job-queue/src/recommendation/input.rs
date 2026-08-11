//! Strict parsing and database attestation for recommendation job inputs.

use crate::resolver::{ResolvedConfig, resolve_config_on};
use crate::runner::ResolveError;
use crate::types::ErrorClass;
use chrono::NaiveDate;
use serde::{Deserialize, Deserializer, Serialize, de};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecommendationPayload {
    pub run_id: Uuid,
    pub strategy_config_id: Uuid,
    #[serde(deserialize_with = "deserialize_date")]
    pub as_of: NaiveDate,
    pub dataset: DatasetPin,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetPin {
    pub id: Uuid,
    #[serde(deserialize_with = "deserialize_nonempty")]
    pub dataset_id: String,
    #[serde(deserialize_with = "deserialize_nonempty")]
    pub version: String,
    #[serde(deserialize_with = "deserialize_positive_u32")]
    pub curated_version: u32,
    #[serde(deserialize_with = "deserialize_sha256")]
    pub manifest_sha256: String,
}

fn deserialize_date<'de, D>(deserializer: D) -> Result<NaiveDate, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    let canonical = value.len() == 10
        && value.as_bytes()[4] == b'-'
        && value.as_bytes()[7] == b'-'
        && value
            .bytes()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit());
    if !canonical {
        return Err(de::Error::custom("date must use YYYY-MM-DD"));
    }
    NaiveDate::parse_from_str(&value, "%Y-%m-%d").map_err(de::Error::custom)
}

fn deserialize_nonempty<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value.trim().is_empty() {
        return Err(de::Error::custom("value must not be empty"));
    }
    Ok(value)
}

fn deserialize_positive_u32<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    let value = u32::deserialize(deserializer)?;
    if value == 0 {
        return Err(de::Error::custom("curated_version must be positive"));
    }
    Ok(value)
}

fn deserialize_sha256<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value.len() != 64
        || !value
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(de::Error::custom(
            "manifest_sha256 must be exactly 64 lowercase hexadecimal characters",
        ));
    }
    Ok(value)
}

impl TryFrom<serde_json::Value> for RecommendationPayload {
    type Error = RecommendationInputError;

    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        serde_json::from_value(value).map_err(|error| RecommendationInputError::Malformed {
            detail: error.to_string(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttestedDatasetStatus {
    Ready,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttestedDataset {
    pub id: Uuid,
    pub dataset_id: String,
    pub version: String,
    pub curated_version: u32,
    pub status: AttestedDatasetStatus,
    pub manifest_sha256: String,
    pub storage_path: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AttestedRecommendationInput {
    pub payload: RecommendationPayload,
    pub resolved_config: ResolvedConfig,
    pub dataset: AttestedDataset,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RecommendationInputError {
    #[error("malformed recommendation payload: {detail}")]
    Malformed { detail: String },
    #[error("recommendation input not found")]
    NotFound,
    #[error("recommendation input integrity failure: {detail}")]
    Integrity { detail: String },
    #[error("recommendation data blocked: {detail}")]
    DataBlocked { detail: String },
    #[error("recommendation input unavailable: {detail}")]
    Unavailable { detail: String },
}

impl RecommendationInputError {
    pub const fn class(&self) -> ErrorClass {
        match self {
            Self::Malformed { .. } | Self::NotFound => ErrorClass::Input,
            Self::Integrity { .. } => ErrorClass::Integrity,
            Self::DataBlocked { .. } => ErrorClass::DataBlocked,
            Self::Unavailable { .. } => ErrorClass::Transient,
        }
    }

    pub const fn code(&self) -> &'static str {
        match self {
            Self::Malformed { .. } => "RECOMMENDATION_INPUT_MALFORMED",
            Self::NotFound => "RECOMMENDATION_INPUT_NOT_FOUND",
            Self::Integrity { .. } => "RECOMMENDATION_INPUT_INTEGRITY",
            Self::DataBlocked { .. } => "RECOMMENDATION_DATA_BLOCKED",
            Self::Unavailable { .. } => "RECOMMENDATION_INPUT_UNAVAILABLE",
        }
    }
}

#[derive(sqlx::FromRow)]
struct RunRow {
    job_id: Option<Uuid>,
    status: String,
    strategy_config_id: Option<Uuid>,
    as_of: NaiveDate,
    dataset_version_id: Option<Uuid>,
    dataset_manifest_sha256: Option<String>,
}

#[derive(sqlx::FromRow)]
struct DatasetRow {
    id: Uuid,
    dataset_id: String,
    version: String,
    status: String,
    manifest_sha256: String,
    storage_path: String,
}

/// Re-attest a parsed payload against the claimed job's owner and current DB rows.
pub async fn attest_recommendation_input(
    pool: &PgPool,
    claimed_job_id: Uuid,
    claimed_owner_user_id: Uuid,
    payload: RecommendationPayload,
) -> Result<AttestedRecommendationInput, RecommendationInputError> {
    let mut transaction = pool
        .begin()
        .await
        .map_err(|error| unavailable("begin attestation", error))?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *transaction)
        .await
        .map_err(|error| unavailable("configure attestation snapshot", error))?;

    let run = sqlx::query_as::<_, RunRow>(
        "SELECT job_id, status, strategy_config_id, as_of, dataset_version_id, \
                    dataset_manifest_sha256 \
             FROM recommendation_runs \
             WHERE id = $1 AND owner_user_id = $2",
    )
    .bind(payload.run_id)
    .bind(claimed_owner_user_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|error| unavailable("read recommendation run", error))?
    .ok_or(RecommendationInputError::NotFound)?;

    require_match(
        run.job_id == Some(claimed_job_id),
        "run job id does not match claim",
    )?;
    require_match(run.status == "PENDING", "run is not PENDING")?;
    require_match(
        run.strategy_config_id == Some(payload.strategy_config_id),
        "run strategy config does not match payload",
    )?;
    require_match(
        run.as_of == payload.as_of,
        "run as-of does not match payload",
    )?;

    let resolved_config = resolve_config_on(
        &mut transaction,
        payload.strategy_config_id,
        claimed_owner_user_id,
    )
    .await
    .map_err(map_resolve_error)?;

    let dataset = sqlx::query_as::<_, DatasetRow>(
        "SELECT id, dataset_id, version, status, manifest_sha256, storage_path \
         FROM dataset_versions WHERE id = $1",
    )
    .bind(payload.dataset.id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|error| unavailable("read dataset version", error))?
    .ok_or_else(|| RecommendationInputError::DataBlocked {
        detail: "pinned dataset version is missing".into(),
    })?;
    let status = match dataset.status.as_str() {
        "READY" => AttestedDatasetStatus::Ready,
        "WARNING" => AttestedDatasetStatus::Warning,
        "BLOCKED" => {
            return Err(RecommendationInputError::DataBlocked {
                detail: "pinned dataset version is BLOCKED".into(),
            });
        }
        _ => {
            return Err(RecommendationInputError::Integrity {
                detail: "dataset has an unknown status".into(),
            });
        }
    };

    require_match(
        run.dataset_version_id == Some(payload.dataset.id),
        "run dataset version does not match payload",
    )?;
    require_match(
        run.dataset_manifest_sha256.as_deref() == Some(payload.dataset.manifest_sha256.as_str()),
        "run manifest hash does not match payload",
    )?;
    require_match(
        dataset.dataset_id == payload.dataset.dataset_id,
        "dataset logical id does not match payload",
    )?;
    require_match(
        dataset.version == payload.dataset.version,
        "dataset version does not match payload",
    )?;
    require_match(
        dataset.manifest_sha256 == payload.dataset.manifest_sha256,
        "dataset manifest hash does not match payload",
    )?;

    transaction
        .commit()
        .await
        .map_err(|error| unavailable("commit attestation", error))?;

    let curated_version = payload.dataset.curated_version;
    Ok(AttestedRecommendationInput {
        payload,
        resolved_config,
        dataset: AttestedDataset {
            id: dataset.id,
            dataset_id: dataset.dataset_id,
            version: dataset.version,
            curated_version,
            status,
            manifest_sha256: dataset.manifest_sha256,
            storage_path: dataset.storage_path,
        },
    })
}

fn require_match(matches: bool, detail: &'static str) -> Result<(), RecommendationInputError> {
    if matches {
        Ok(())
    } else {
        Err(RecommendationInputError::Integrity {
            detail: detail.into(),
        })
    }
}

fn map_resolve_error(error: ResolveError) -> RecommendationInputError {
    match error {
        ResolveError::NotFound(_) => RecommendationInputError::NotFound,
        ResolveError::Unknown(detail) => RecommendationInputError::Integrity { detail },
        ResolveError::Unavailable(detail) => RecommendationInputError::Unavailable { detail },
    }
}

fn unavailable(context: &'static str, error: sqlx::Error) -> RecommendationInputError {
    RecommendationInputError::Unavailable {
        detail: format!("{context}: {error}"),
    }
}
