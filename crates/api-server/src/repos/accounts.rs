//! Actor-scoped repository over `accounts` (tenant table, `owner_user_id`,
//! FORCE RLS).

use crate::actor_tx::{actor_uuid, begin_actor_tx};
use crate::error::{TenancyError, TenancyResult};
use auth::entitlement::Actor;
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// A row of `accounts` as the actor may see it.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct AccountRow {
    pub id: Uuid,
    pub owner_user_id: Uuid,
    pub account_type: String,
    pub name: String,
    pub currency: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating an account. Ownership is derived from the actor only.
#[derive(Debug, Clone)]
pub struct NewAccount {
    pub account_type: String,
    pub name: String,
    pub currency: String,
}

/// Typed repository over `accounts`.
#[derive(Debug, Clone)]
pub struct AccountRepo {
    pool: sqlx::PgPool,
}

impl AccountRepo {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    /// Create an account owned by `actor`.
    pub async fn create(&self, actor: &Actor, input: NewAccount) -> TenancyResult<AccountRow> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let row = sqlx::query_as::<_, AccountRow>(
            "INSERT INTO accounts (owner_user_id, account_type, name, currency) \
             VALUES ($1, $2, $3, $4) \
             RETURNING id, owner_user_id, account_type, name, currency, status, \
                       created_at, updated_at",
        )
        .bind(actor_uuid(actor)?)
        .bind(&input.account_type)
        .bind(&input.name)
        .bind(&input.currency)
        .fetch_one(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(row)
    }

    /// Fetch one of `actor`'s accounts by id; a foreign row => NotFound.
    pub async fn get(&self, actor: &Actor, id: Uuid) -> TenancyResult<AccountRow> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let row = sqlx::query_as::<_, AccountRow>(
            "SELECT id, owner_user_id, account_type, name, currency, status, \
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
                    created_at, updated_at \
             FROM accounts ORDER BY created_at",
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(rows)
    }
}
