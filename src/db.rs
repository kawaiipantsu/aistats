//! Local SQLite cache of normalized usage/cost data.
//!
//! A single-user CLI tool has no business running a server-backed database;
//! SQLite gives durable, query-able local storage with zero operational
//! overhead. Provider API responses are normalized and upserted here so that
//! reporting (hourly/daily/monthly/all-time rollups) is a fast local query
//! regardless of how each provider's API chooses to bucket its own data.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};

use crate::models::{CostRecord, UsageRecord};

pub fn open(path: &std::path::Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)
        .with_context(|| format!("failed to open database at {}", path.display()))?;
    conn.execute_batch(
        "
        PRAGMA journal_mode = WAL;
        PRAGMA foreign_keys = ON;

        CREATE TABLE IF NOT EXISTS usage_records (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            provider TEXT NOT NULL,
            model TEXT NOT NULL,
            bucket_start TEXT NOT NULL,
            bucket_end TEXT NOT NULL,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cached_input_tokens INTEGER NOT NULL DEFAULT 0,
            cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
            requests INTEGER NOT NULL DEFAULT 0,
            cost_usd REAL,
            source TEXT NOT NULL,
            UNIQUE(provider, model, bucket_start, bucket_end, source)
        );
        CREATE INDEX IF NOT EXISTS idx_usage_bucket ON usage_records(bucket_start);
        CREATE INDEX IF NOT EXISTS idx_usage_provider ON usage_records(provider);

        CREATE TABLE IF NOT EXISTS cost_records (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            provider TEXT NOT NULL,
            model TEXT,
            bucket_start TEXT NOT NULL,
            bucket_end TEXT NOT NULL,
            amount_usd REAL NOT NULL,
            description TEXT,
            source TEXT NOT NULL,
            UNIQUE(provider, model, bucket_start, bucket_end, description, source)
        );
        CREATE INDEX IF NOT EXISTS idx_cost_bucket ON cost_records(bucket_start);

        CREATE TABLE IF NOT EXISTS sync_state (
            provider TEXT PRIMARY KEY,
            last_synced_until TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        ",
    )
    .context("failed to initialize database schema")?;
    Ok(conn)
}

pub fn upsert_usage_records(conn: &mut Connection, records: &[UsageRecord]) -> Result<usize> {
    let tx = conn.transaction()?;
    let mut count = 0;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO usage_records
                (provider, model, bucket_start, bucket_end, input_tokens, output_tokens,
                 cached_input_tokens, cache_creation_tokens, requests, cost_usd, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(provider, model, bucket_start, bucket_end, source) DO UPDATE SET
                input_tokens = excluded.input_tokens,
                output_tokens = excluded.output_tokens,
                cached_input_tokens = excluded.cached_input_tokens,
                cache_creation_tokens = excluded.cache_creation_tokens,
                requests = excluded.requests,
                cost_usd = excluded.cost_usd",
        )?;
        for r in records {
            stmt.execute(params![
                r.provider,
                r.model,
                r.bucket_start.to_rfc3339(),
                r.bucket_end.to_rfc3339(),
                r.input_tokens,
                r.output_tokens,
                r.cached_input_tokens,
                r.cache_creation_tokens,
                r.requests,
                r.cost_usd,
                r.source,
            ])?;
            count += 1;
        }
    }
    tx.commit()?;
    Ok(count)
}

pub fn upsert_cost_records(conn: &mut Connection, records: &[CostRecord]) -> Result<usize> {
    let tx = conn.transaction()?;
    let mut count = 0;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO cost_records
                (provider, model, bucket_start, bucket_end, amount_usd, description, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(provider, model, bucket_start, bucket_end, description, source) DO UPDATE SET
                amount_usd = excluded.amount_usd",
        )?;
        for r in records {
            stmt.execute(params![
                r.provider,
                r.model,
                r.bucket_start.to_rfc3339(),
                r.bucket_end.to_rfc3339(),
                r.amount_usd,
                r.description,
                r.source,
            ])?;
            count += 1;
        }
    }
    tx.commit()?;
    Ok(count)
}

pub fn query_usage(
    conn: &Connection,
    since: DateTime<Utc>,
    until: DateTime<Utc>,
    provider: Option<&str>,
) -> Result<Vec<UsageRecord>> {
    let sql = "SELECT provider, model, bucket_start, bucket_end, input_tokens, output_tokens,
                      cached_input_tokens, cache_creation_tokens, requests, cost_usd, source
               FROM usage_records
               WHERE bucket_start < ?2 AND bucket_end > ?1
                 AND (?3 IS NULL OR provider = ?3)
               ORDER BY bucket_start ASC";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(
        params![since.to_rfc3339(), until.to_rfc3339(), provider],
        |row| {
            Ok(UsageRecord {
                provider: row.get(0)?,
                model: row.get(1)?,
                bucket_start: parse_dt(row.get::<_, String>(2)?),
                bucket_end: parse_dt(row.get::<_, String>(3)?),
                input_tokens: row.get(4)?,
                output_tokens: row.get(5)?,
                cached_input_tokens: row.get(6)?,
                cache_creation_tokens: row.get(7)?,
                requests: row.get(8)?,
                cost_usd: row.get(9)?,
                source: row.get(10)?,
            })
        },
    )?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .context("failed to read usage_records")
}

pub fn query_cost(
    conn: &Connection,
    since: DateTime<Utc>,
    until: DateTime<Utc>,
    provider: Option<&str>,
) -> Result<Vec<CostRecord>> {
    let sql = "SELECT provider, model, bucket_start, bucket_end, amount_usd, description, source
               FROM cost_records
               WHERE bucket_start < ?2 AND bucket_end > ?1
                 AND (?3 IS NULL OR provider = ?3)
               ORDER BY bucket_start ASC";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(
        params![since.to_rfc3339(), until.to_rfc3339(), provider],
        |row| {
            Ok(CostRecord {
                provider: row.get(0)?,
                model: row.get(1)?,
                bucket_start: parse_dt(row.get::<_, String>(2)?),
                bucket_end: parse_dt(row.get::<_, String>(3)?),
                amount_usd: row.get(4)?,
                description: row.get(5)?,
                source: row.get(6)?,
            })
        },
    )?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .context("failed to read cost_records")
}

pub fn get_sync_state(conn: &Connection, provider: &str) -> Result<Option<DateTime<Utc>>> {
    let mut stmt = conn.prepare("SELECT last_synced_until FROM sync_state WHERE provider = ?1")?;
    let mut rows = stmt.query(params![provider])?;
    if let Some(row) = rows.next()? {
        let s: String = row.get(0)?;
        Ok(Some(parse_dt(s)))
    } else {
        Ok(None)
    }
}

pub fn set_sync_state(conn: &Connection, provider: &str, until: DateTime<Utc>) -> Result<()> {
    conn.execute(
        "INSERT INTO sync_state (provider, last_synced_until, updated_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(provider) DO UPDATE SET
            last_synced_until = excluded.last_synced_until,
            updated_at = excluded.updated_at",
        params![provider, until.to_rfc3339(), Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

fn parse_dt(s: String) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(&s)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}
