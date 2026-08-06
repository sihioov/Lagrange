//! Read-only catalog over the shared strategy metadata tables
//! (`strategies`, `strategy_versions`, `strategy_parameter_schemas`).

use crate::error::{TenancyError, TenancyResult};
use auth::entitlement::Actor;
use serde_json::Value;
use sqlx::FromRow;

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct StrategyRow {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub risk_description: String,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct StrategyVersionRow {
    pub version: String,
    pub required_factors: Value,
    pub min_lookback: Option<i32>,
    pub supported_market: String,
    pub cadence: String,
}

/// Read-only repository over the shared strategy registry.
#[derive(Debug, Clone)]
pub struct StrategyCatalogRepo {
    pool: sqlx::PgPool,
}

impl StrategyCatalogRepo {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    pub async fn list(&self, _actor: &Actor) -> TenancyResult<Vec<StrategyRow>> {
        let rows = sqlx::query_as::<_, StrategyRow>(
            "SELECT id, display_name, description, risk_description, state \
             FROM strategies ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(TenancyError::from_sqlx)?;
        Ok(rows)
    }

    pub async fn get(&self, _actor: &Actor, id: &str) -> TenancyResult<StrategyRow> {
        let row = sqlx::query_as::<_, StrategyRow>(
            "SELECT id, display_name, description, risk_description, state \
             FROM strategies WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(TenancyError::from_sqlx)?;
        crate::error::map_optional(row)
    }

    /// The latest published version of a strategy (immutable once published).
    pub async fn latest_version(
        &self,
        _actor: &Actor,
        strategy_id: &str,
    ) -> TenancyResult<Option<String>> {
        let version: Option<String> = sqlx::query_scalar(
            "SELECT version FROM strategy_versions WHERE strategy_id = $1 \
             ORDER BY created_at DESC, version DESC LIMIT 1",
        )
        .bind(strategy_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(TenancyError::from_sqlx)?;
        Ok(version)
    }

    /// One published version of a strategy.
    pub async fn get_version(
        &self,
        _actor: &Actor,
        strategy_id: &str,
        version: &str,
    ) -> TenancyResult<StrategyVersionRow> {
        let row = sqlx::query_as::<_, StrategyVersionRow>(
            "SELECT version, required_factors, min_lookback, supported_market, cadence \
             FROM strategy_versions WHERE strategy_id = $1 AND version = $2",
        )
        .bind(strategy_id)
        .bind(version)
        .fetch_optional(&self.pool)
        .await
        .map_err(TenancyError::from_sqlx)?;
        crate::error::map_optional(row)
    }

    /// The published JSON Schema for a strategy version (member parameters
    /// are schema-bound; arbitrary code is never accepted).
    pub async fn param_schema(
        &self,
        _actor: &Actor,
        strategy_id: &str,
        version: &str,
    ) -> TenancyResult<Value> {
        let schema: Option<Value> = sqlx::query_scalar(
            "SELECT schema_json FROM strategy_parameter_schemas \
             WHERE strategy_id = $1 AND version = $2",
        )
        .bind(strategy_id)
        .bind(version)
        .fetch_optional(&self.pool)
        .await
        .map_err(TenancyError::from_sqlx)?;
        schema.ok_or(TenancyError::NotFound)
    }
}
