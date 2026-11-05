use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde_json::json;

use crate::cli::FunArgs;
use crate::commands::report_cmd::write_text;
use crate::config::Config;
use crate::fun::{self, FunReport, Range, Scope};
use crate::paths;
use crate::report::{OutputFormat, fun as render};
use crate::time_util::resolve_range;

/// Webhooks allow about five requests per two seconds, and a report is rarely
/// more than a handful of messages, so a fixed pause is enough.
const WEBHOOK_PAUSE: Duration = Duration::from_millis(600);

const WEBHOOK_TIMEOUT: Duration = Duration::from_secs(20);

pub async fn run(cfg: &Config, args: &FunArgs) -> Result<()> {
    let format = OutputFormat::parse(&args.format)?;
    if !matches!(
        format,
        OutputFormat::Text
            | OutputFormat::Markdown
            | OutputFormat::Html
            | OutputFormat::Json
            | OutputFormat::Discord
    ) {
        bail!(
            "--fun-stats supports --format text|markdown|html|json|discord, not '{}'",
            args.format
        );
    }
    if args.send && format != OutputFormat::Discord {
        bail!("--send requires --format discord");
    }
    if args.public_url.is_some() && format != OutputFormat::Html {
        bail!("--public-url requires --format html");
    }
    let html_options = render::HtmlOptions::new(args.public_url.as_deref())?;
    if args.public_url.is_some() {
        let output = args
            .output
            .as_deref()
            .context("--public-url requires --output")?;
        if output
            .file_name()
            .is_some_and(|name| name == render::SOCIAL_IMAGE_NAME)
        {
            bail!(
                "--output must not use the reserved social image filename {}",
                render::SOCIAL_IMAGE_NAME
            );
        }
    }

    let (since, until) = resolve_range(
        &args.period,
        args.from.as_deref(),
        args.to.as_deref(),
        Utc::now(),
    )?;

    let report = fun::scan_hosts(
        Range { since, until },
        load_salt(),
        &args.ssh_hosts,
        args.ssh_key.as_deref(),
        Duration::from_secs(args.ssh_timeout),
    )
    .await?;
    print_scope_notice(&report);

    if report.total.counts.sessions == 0 {
        eprintln!("No Claude Code or Codex sessions found in that period.");
        // An explicit HTML export should still produce a usable page.
        if format != OutputFormat::Html {
            return Ok(());
        }
    }

    let top = args.top.max(1);
    if args.send {
        return send(cfg, args.webhook.as_deref(), &report, top).await;
    }

    let rendered = match format {
        OutputFormat::Markdown => render::render_markdown(&report, top),
        OutputFormat::Html => render::render_html(&report, top, &html_options),
        OutputFormat::Json => render::render_json(&report)?,
        OutputFormat::Discord => render::render_discord(&report, top),
        _ => render::render_text(&report, top),
    };
    if args.public_url.is_some() {
        let output = args
            .output
            .as_deref()
            .context("--public-url requires --output")?;
        let image_path = output.with_file_name(render::SOCIAL_IMAGE_NAME);
        std::fs::write(&image_path, render::SOCIAL_IMAGE)
            .with_context(|| format!("failed to write social image {}", image_path.display()))?;
    }
    write_text(args.output.as_deref(), &rendered)
}

fn print_scope_notice(report: &FunReport) {
    let coverage = &report.coverage;
    if coverage.hosts_scanned > 1 {
        eprintln!(
            "Scanned {} hosts (all users on {}, signed-in account only on {}).",
            coverage.hosts_scanned,
            coverage.hosts_all_users,
            coverage.hosts_scanned - coverage.hosts_all_users,
        );
    } else if report.scope == Scope::CurrentUser {
        eprintln!("Not running as root: only your own logs were scanned.");
    }
    if coverage.files_unreadable > 0 {
        eprintln!(
            "Skipped {} log files that could not be read.",
            coverage.files_unreadable
        );
    }
    if coverage.records_malformed + coverage.records_oversized > 0 {
        eprintln!(
            "Skipped {} malformed and {} oversized log records.",
            coverage.records_malformed, coverage.records_oversized
        );
    }
}

/// Loads the salt behind user pseudonyms, creating it on first use so a
/// person keeps the same pseudonym across runs. Failing to save it only costs
/// that stability, so it never stops the report.
fn load_salt() -> u128 {
    let Ok(path) = fun::salt_path() else {
        return fun::random_salt();
    };
    if let Some(salt) = read_salt(&path) {
        return salt;
    }
    let salt = fun::random_salt();
    let saved = paths::ensure_dirs()
        .and_then(|()| {
            std::fs::write(&path, format!("{salt:032x}\n"))
                .with_context(|| format!("failed to write {}", path.display()))
        })
        .and_then(|()| paths::restrict_permissions(&path));
    if let Err(e) = saved {
        tracing::debug!("could not save pseudonym salt: {e:#}");
    }
    salt
}

fn read_salt(path: &Path) -> Option<u128> {
    let text = std::fs::read_to_string(path).ok()?;
    u128::from_str_radix(text.trim(), 16).ok()
}

async fn send(cfg: &Config, webhook: Option<&str>, report: &FunReport, top: usize) -> Result<()> {
    let webhook = webhook
        .map(str::to_string)
        .or_else(|| crate::config::resolve_discord_webhook(cfg))
        .context("no Discord webhook configured; pass --webhook or set discord.webhook_url / AI_STATS_DISCORD_WEBHOOK_URL")?;

    let client = reqwest::Client::builder()
        .timeout(WEBHOOK_TIMEOUT)
        .build()
        .context("failed to build HTTP client")?;
    let messages = render::discord_messages(report, top);
    for (index, message) in messages.iter().enumerate() {
        if index > 0 {
            tokio::time::sleep(WEBHOOK_PAUSE).await;
        }
        let payload = json!({
            "content": message,
            "allowed_mentions": {"parse": []},
        });
        let resp = client
            .post(&webhook)
            .json(&payload)
            .send()
            .await
            // A webhook URL contains its token, so keep it out of the error.
            .map_err(reqwest::Error::without_url)
            .context("failed to send Discord webhook")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Discord webhook returned HTTP {status}: {body}");
        }
    }
    println!(
        "Sent fun stats to Discord webhook ({} messages).",
        messages.len()
    );
    Ok(())
}
