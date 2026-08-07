//! Actor-scoped repository over `accounts` and `account_strategy_bindings`
//! (tenant tables, `owner_user_id`, FORCE RLS).
//!
//! `accounts.initial_cash`/`cost_profile_*` describe how the account was
//! opened; current cash is never cached here -- it is always derived by
//! replaying `cash_ledger` (Todo 18's shared ledger contract). Creating a
//! PAPER account therefore seeds exactly one `cash_ledger` row (the opening
//! `DEPOSIT`) in the SAME transaction as the account row, so an account can
//! never exist without its funding event.

use crate::actor_tx::{actor_uuid, begin_actor_tx};
use crate::error::{TenancyError, TenancyResult};
use auth::entitlement::Actor;
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// A row of `accounts` as the actor may see it.
#[derive(Debug, Clone, PartialEq, FromRow)]
pub struct AccountRow {
    pub id: Uuid,
    pub owner_user_id: Uuid,
    pub account_type: String,
    pub name: String,
    pub currency: String,
    pub status: String,
    pub initial_cash: Option<String>,
    pub cost_profile_id: String,
    pub cost_profile_version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating an account. Ownership is derived from the actor only.
#[derive(Debug, Clone)]
pub struct NewAccount {
    pub account_type: String,
    pub name: String,
    pub currency: String,
    /// Required (and validated positive) for PAPER accounts by the caller
    /// before this repository is ever reached; LIVE accounts (Phase 3) are
    /// funded by the connected broker instead.
    pub initial_cash: Option<String>,
    pub cost_profile_id: String,
    pub cost_profile_version: i32,
}

/// One row of `account_strategy_bindings`: an immutable binding record.
/// `unbound_at` is the ONLY field ever mutated after insert, and only once,
/// to close the binding out when a new one replaces it.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct BindingRow {
    pub id: Uuid,
    pub account_id: Uuid,
    pub strategy_config_id: Uuid,
    pub strategy_id: String,
    pub strategy_version: String,
    pub bound_at: DateTime<Utc>,
    pub unbound_at: Option<DateTime<Utc>>,
}

/// Typed repository over `accounts` and `account_strategy_bindings`.
#[derive(Debug, Clone)]
pub struct AccountRepo {
    pool: sqlx::PgPool,
}

impl AccountRepo {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    /// Create an account owned by `actor`. For a PAPER account with
    /// `initial_cash` set, seeds the opening `cash_ledger` DEPOSIT (seq 1)
    /// in the same transaction, so the account and its funding event are
    /// atomic -- there is never an account with no ledger history.
    pub async fn create(&self, actor: &Actor, input: NewAccount) -> TenancyResult<AccountRow> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let owner = actor_uuid(actor)?;
        let row = sqlx::query_as::<_, AccountRow>(
            "INSERT INTO accounts \
             (owner_user_id, account_type, name, currency, initial_cash, \
              cost_profile_id, cost_profile_version) \
             VALUES ($1, $2, $3, $4, $5::numeric, $6, $7) \
             RETURNING id, owner_user_id, account_type, name, currency, status, \
                       initial_cash::text, cost_profile_id, cost_profile_version, \
                       created_at, updated_at",
        )
        .bind(owner)
        .bind(&input.account_type)
        .bind(&input.name)
        .bind(&input.currency)
        .bind(&input.initial_cash)
        .bind(&input.cost_profile_id)
        .bind(input.cost_profile_version)
        .fetch_one(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        if let Some(initial) = &input.initial_cash {
            sqlx::query(
                "INSERT INTO cash_ledger \
                 (account_id, owner_user_id, seq, event_type, amount, balance, currency) \
                 VALUES ($1, $2, 1, 'DEPOSIT', $3::numeric, $3::numeric, $4)",
            )
            .bind(row.id)
            .bind(owner)
            .bind(initial)
            .bind(&input.currency)
            .execute(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?;
        }
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(row)
    }

    /// Fetch one of `actor`'s accounts by id; a foreign row => NotFound.
    pub async fn get(&self, actor: &Actor, id: Uuid) -> TenancyResult<AccountRow> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let row = sqlx::query_as::<_, AccountRow>(
            "SELECT id, owner_user_id, account_type, name, currency, status, \
                    initial_cash::text, cost_profile_id, cost_profile_version, \
                    created_at, updated_at \
             FROM accounts WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        crate::error::map_optional(row)
    }

    /// List `actor`'s own accounts.
    pub async fn list(&self, actor: &Actor) -> TenancyResult<Vec<AccountRow>> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let rows = sqlx::query_as::<_, AccountRow>(
            "SELECT id, owner_user_id, account_type, name, currency, status, \
                    initial_cash::text, cost_profile_id, cost_profile_version, \
                    created_at, updated_at \
             FROM accounts ORDER BY created_at",
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(rows)
    }

    /// The account's current ACTIVE binding, if any.
    pub async fn active_binding(
        &self,
        actor: &Actor,
        account_id: Uuid,
    ) -> TenancyResult<Option<BindingRow>> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let row = sqlx::query_as::<_, BindingRow>(
            "SELECT id, account_id, strategy_config_id, strategy_id, strategy_version, \
                    bound_at, unbound_at \
             FROM account_strategy_bindings WHERE account_id = $1 AND unbound_at IS NULL",
        )
        .bind(account_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(row)
    }

    /// The account's full binding history, oldest first (immutable; nothing
    /// here is ever deleted).
    pub async fn binding_history(
        &self,
        actor: &Actor,
        account_id: Uuid,
    ) -> TenancyResult<Vec<BindingRow>> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let rows = sqlx::query_as::<_, BindingRow>(
            "SELECT id, account_id, strategy_config_id, strategy_id, strategy_version, \
                    bound_at, unbound_at \
             FROM account_strategy_bindings WHERE account_id = $1 ORDER BY bound_at",
        )
        .bind(account_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(rows)
    }

    /// Binds `account_id` to a strategy config, closing out any existing
    /// active binding first -- both in the SAME transaction, so the account
    /// is never observed with two active bindings or zero bindings mid-
    /// switch. This is FR-PAPER-004's "branch on strategy change": the OLD
    /// binding's row is never edited except for `unbound_at`, and a brand
    /// new row records the new strategy identity, so execution history
    /// never mixes strategy versions.
    pub async fn bind_strategy(
        &self,
        actor: &Actor,
        account_id: Uuid,
        strategy_config_id: Uuid,
        strategy_id: &str,
        strategy_version: &str,
    ) -> TenancyResult<BindingRow> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        sqlx::query(
            "UPDATE account_strategy_bindings SET unbound_at = now() \
             WHERE account_id = $1 AND unbound_at IS NULL",
        )
        .bind(account_id)
        .execute(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        let row = sqlx::query_as::<_, BindingRow>(
            "INSERT INTO account_strategy_bindings \
             (account_id, owner_user_id, strategy_config_id, strategy_id, strategy_version) \
             VALUES ($1, $2, $3, $4, $5) \
             RETURNING id, account_id, strategy_config_id, strategy_id, strategy_version, \
                       bound_at, unbound_at",
        )
        .bind(account_id)
        .bind(actor_uuid(actor)?)
        .bind(strategy_config_id)
        .bind(strategy_id)
        .bind(strategy_version)
        .fetch_one(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(row)
    }
}
