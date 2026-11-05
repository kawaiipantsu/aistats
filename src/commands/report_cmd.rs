use std::path::Path;

use anyhow::{Context, Result, bail};
use chrono::Utc;
use rusqlite::Connection;

use crate::config::Config;
use crate::models::Granularity;
use crate::report::{self, OutputFormat, Rendered};
use crate::time_util::{auto_granularity, resolve_range};

#[allow(clippy::too_many_arguments)]
pub async fn run(
    cfg: &Config,
    conn: &Connection,
    provider: Option<&str>,
    period: &str,
    from: Option<&str>,
    to: Option<&str>,
    group_by: &str,
    format: &str,
    output: Option<&Path>,
    top: usize,
    send: bool,
    webhook_override: Option<&str>,
) -> Result<()> {
    let (since, until) = resolve_range(period, from, to, Utc::now())?;

    let granularity = if group_by == "auto" {
        auto_granularity(since, until)
    } else {
        group_by.parse::<Granularity>()?
    };

    let out_format = OutputFormat::parse(format)?;

    let rep =
        crate::aggregate::build_report(conn, since, until, provider, granularity, &cfg.pricing)?;
    let rendered = report::render(&rep, out_format, top.max(1))?;

    if out_format == OutputFormat::Discord && send {
        let webhook = webhook_override
            .map(str::to_string)
            .or_else(|| crate::config::resolve_discord_webhook(cfg))
            .context("no Discord webhook configured; pass --webhook or set discord.webhook_url / AI_STATS_DISCORD_WEBHOOK_URL")?;
        let payload = report::discord::build_payload(&rep, top.max(1));
        let client = reqwest::Client::new();
        let resp = client
            .post(&webhook)
            .json(&payload)
            .send()
            .await
            .context("failed to send Discord webhook")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Discord webhook returned HTTP {status}: {body}");
        }
        println!("Sent report to Discord webhook.");
        return Ok(());
    }

    match rendered {
        Rendered::Text(text) => write_text(output, &text)?,
        Rendered::Bytes(bytes) => write_bytes(output, &bytes)?,
    }

    Ok(())
}

pub(crate) fn write_text(output: Option<&Path>, text: &str) -> Result<()> {
    match output {
        Some(path) => {
            std::fs::write(path, text)
                .with_context(|| format!("failed to write {}", path.display()))?;
            println!("Wrote report to {}", path.display());
        }
        None => print!("{text}"),
    }
    Ok(())
}

fn write_bytes(output: Option<&Path>, bytes: &[u8]) -> Result<()> {
    match output {
        Some(path) => {
            std::fs::write(path, bytes)
                .with_context(|| format!("failed to write {}", path.display()))?;
            println!("Wrote report to {}", path.display());
        }
        None => bail!("binary formats (pdf) require --output <file>"),
    }
    Ok(())
}
