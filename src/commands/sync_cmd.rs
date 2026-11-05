use anyhow::Result;
use chrono::{Duration, Utc};
use rusqlite::Connection;

use crate::config::Config;
use crate::db;
use crate::time_util::parse_timestamp;

use super::{enabled_providers, find_provider, require_known_provider};

pub async fn run(
    cfg: &Config,
    conn: &mut Connection,
    provider: Option<&str>,
    since: Option<&str>,
    until: Option<&str>,
) -> Result<()> {
    let until_dt = match until {
        Some(s) => parse_timestamp(s)?,
        None => Utc::now(),
    };

    let targets = if let Some(id) = provider {
        require_known_provider(id)?;
        vec![find_provider(cfg, id)?]
    } else {
        enabled_providers(cfg)
    };

    if targets.is_empty() {
        println!("No providers are enabled with valid credentials. See `ai-stats providers list`.");
        return Ok(());
    }

    for p in targets {
        let since_dt = match since {
            Some(s) => parse_timestamp(s)?,
            None => db::get_sync_state(conn, p.id())?.unwrap_or(until_dt - Duration::days(7)),
        };

        if since_dt >= until_dt {
            println!("{}: nothing to do (since >= until)", p.display_name());
            continue;
        }

        println!(
            "Syncing {} from {} to {} (UTC)...",
            p.display_name(),
            since_dt.format("%Y-%m-%d %H:%M"),
            until_dt.format("%Y-%m-%d %H:%M")
        );

        match p.fetch_usage(since_dt, until_dt).await {
            Ok(result) => {
                let n_usage = db::upsert_usage_records(conn, &result.usage)?;
                let n_cost = db::upsert_cost_records(conn, &result.cost)?;
                db::set_sync_state(conn, p.id(), until_dt)?;
                println!("  stored {n_usage} usage row(s), {n_cost} cost row(s)");
            }
            Err(e) => {
                eprintln!("  FAILED: {e:#}");
            }
        }
    }

    Ok(())
}
