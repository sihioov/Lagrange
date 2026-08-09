//! Actor-scoped reads of the Paper ledger views: `orders`, `positions`, and
//! `daily_equity` for one owned account (tenant tables, FORCE RLS). Writes
//! are the Paper scheduler's (Todos 30-32); the API is read-only here.

use crate::actor_tx::begin_actor_tx;
use crate::error::{TenancyError, TenancyResult};
use crate::http::pagination::Cursor;
use crate::repos::split_page;
use auth::entitlement::Actor;
use chrono::{DateTime, NaiveDate, Utc};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct OrderRow {
    pub id: Uuid,
    pub order_ref: String,
    pub instrument_id: String,
    pub side: String,
    pub quantity: String,
    pub price: Option<String>,
    pub status: String,
    pub submitted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct PositionRow {
    pub instrument_id: String,
    pub quantity: String,
    pub avg_price: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct EquityRow {
    pub trading_date: NaiveDate,
    pub equity: String,
    pub cash: String,
    pub positions_value: String,
    pub currency: String,
    /// Whether `cash` agrees with `cash_ledger` as of this date.
    ///
    /// `accounts.rs` states the rule this system runs on: "current cash is
    /// never cached here -- it is always derived by replaying `cash_ledger`".
    /// `daily_equity.cash` is exactly such a cache, and nothing checked it
    /// against the authority before serving it. `false` here means this row
    /// is a stored number nobody has proven agrees with the ledger -- serve
    /// it, but say so, rather than presenting it with the same confidence as
    /// a reconciled figure.
    pub cash_reconciled: bool,
}

/// Read-only repository over the Paper ledger tables.
#[derive(Debug, Clone)]
pub struct PaperRepo {
    pool: sqlx::PgPool,
}

impl PaperRepo {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    /// The actor's orders for one owned account (paginated by created_at).
    pub async fn orders(
        &self,
        actor: &Actor,
        account_id: Uuid,
        after: Option<&Cursor>,
        limit: usize,
    ) -> TenancyResult<(Vec<OrderRow>, Option<Cursor>)> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let sql = match after {
            Some(_) => {
                "SELECT id, order_ref, instrument_id, side, quantity::text, price::text, \
                        status, submitted_at, created_at \
                 FROM orders WHERE account_id = $1 AND (created_at, id) > ($2::timestamptz, $3::uuid) \
                 ORDER BY created_at, id LIMIT $4"
            }
            None => {
                "SELECT id, order_ref, instrument_id, side, quantity::text, price::text, \
                        status, submitted_at, created_at \
                 FROM orders WHERE account_id = $1 \
                 ORDER BY created_at, id LIMIT $2"
            }
        };
        let mut q = sqlx::query_as::<_, OrderRow>(sql).bind(account_id);
        if let Some(c) = after {
            q = q.bind(c.k.clone()).bind(parse_cursor_id(c)?);
        }
        let rows = q
            .bind(limit as i64 + 1)
            .fetch_all(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(split_page(rows, limit, |r| {
            (r.created_at.to_rfc3339(), r.id.to_string())
        }))
    }

    pub async fn positions(
        &self,
        actor: &Actor,
        account_id: Uuid,
    ) -> TenancyResult<Vec<PositionRow>> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let rows = sqlx::query_as::<_, PositionRow>(
            "SELECT instrument_id, quantity::text, avg_price::text, updated_at \
             FROM positions WHERE account_id = $1 ORDER BY instrument_id",
        )
        .bind(account_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(rows)
    }

    /// Daily equity points (paginated by trading_date).
    ///
    /// `cash_reconciled` is computed against `cash_ledger` AS OF each row's
    /// own `trading_date` -- the latest ledger balance whose event happened
    /// on or before that date -- via a lateral join, so a historical point is
    /// checked against what the ledger said back then, not against today's
    /// balance. A date with no ledger event yet (impossible for a real
    /// account, since `AccountRepo::create` seeds the opening deposit
    /// atomically with the account) reconciles as `false` rather than being
    /// silently treated as agreeing with nothing.
    pub async fn equity(
        &self,
        actor: &Actor,
        account_id: Uuid,
        after: Option<&Cursor>,
        limit: usize,
    ) -> TenancyResult<(Vec<EquityRow>, Option<Cursor>)> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let sql = match after {
            Some(_) => {
                "SELECT de.trading_date, de.equity::text, de.cash::text, \
                        de.positions_value::text, de.currency, \
                        (cl.balance IS NOT NULL AND cl.balance = de.cash) AS cash_reconciled \
                 FROM daily_equity de \
                 LEFT JOIN LATERAL ( \
                     SELECT balance FROM cash_ledger \
                     WHERE account_id = de.account_id AND ts::date <= de.trading_date \
                     ORDER BY seq DESC LIMIT 1 \
                 ) cl ON true \
                 WHERE de.account_id = $1 AND de.trading_date > $2::date \
                 ORDER BY de.trading_date LIMIT $3"
            }
            None => {
                "SELECT de.trading_date, de.equity::text, de.cash::text, \
                        de.positions_value::text, de.currency, \
                        (cl.balance IS NOT NULL AND cl.balance = de.cash) AS cash_reconciled \
                 FROM daily_equity de \
                 LEFT JOIN LATERAL ( \
                     SELECT balance FROM cash_ledger \
                     WHERE account_id = de.account_id AND ts::date <= de.trading_date \
                     ORDER BY seq DESC LIMIT 1 \
                 ) cl ON true \
                 WHERE de.account_id = $1 \
                 ORDER BY de.trading_date LIMIT $2"
            }
        };
        let mut q = sqlx::query_as::<_, EquityRow>(sql).bind(account_id);
        if let Some(c) = after {
            q = q.bind(c.k.clone());
        }
        let rows = q
            .bind(limit as i64 + 1)
            .fetch_all(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(split_page(rows, limit, |r| {
            (r.trading_date.to_string(), r.trading_date.to_string())
        }))
    }
}

fn parse_cursor_id(c: &Cursor) -> TenancyResult<Uuid> {
    Uuid::parse_str(&c.i).map_err(|_| TenancyError::NotFound)
}
