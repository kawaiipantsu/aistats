//! Generic CSV/JSON importer for usage data pulled manually from consoles
//! that don't offer a programmatic usage API for API-key accounts — notably
//! Google Gemini/Vertex AI and xAI today. Export a billing/usage table from
//! the provider's dashboard, map it to the columns below, and import it so
//! it shows up in reports alongside the API-integrated providers.
//!
//! Expected columns/fields (extra columns are ignored):
//!
//! ```text
//! provider, model, date, input_tokens, output_tokens, requests, cost_usd, cached_input_tokens
//! ```
//!
//! `date` accepts `YYYY-MM-DD` (treated as a whole UTC day) or a full
//! RFC3339 timestamp. `provider` may be omitted from the file if `--provider`
//! is passed on the command line instead.

use std::path::Path;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;

use crate::models::{CostRecord, UsageRecord};
use crate::providers::FetchResult;
use crate::time_util::parse_timestamp;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportFormat {
    Csv,
    Json,
}

impl ImportFormat {
    pub fn from_path(path: &Path) -> Result<Self> {
        match path.extension().and_then(|e| e.to_str()) {
            Some("csv") => Ok(ImportFormat::Csv),
            Some("json") => Ok(ImportFormat::Json),
            other => {
                bail!("cannot infer import format from extension {other:?}; pass --format csv|json")
            }
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "csv" => Ok(ImportFormat::Csv),
            "json" => Ok(ImportFormat::Json),
            other => bail!("unknown import format '{other}' (expected csv|json)"),
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawRow {
    #[serde(default)]
    provider: Option<String>,
    model: String,
    date: String,
    #[serde(default)]
    input_tokens: i64,
    #[serde(default)]
    output_tokens: i64,
    #[serde(default)]
    requests: i64,
    #[serde(default)]
    cost_usd: Option<f64>,
    #[serde(default)]
    cached_input_tokens: i64,
}

pub fn import_file(
    path: &Path,
    format: ImportFormat,
    provider_override: Option<&str>,
) -> Result<FetchResult> {
    let raw_rows: Vec<RawRow> = match format {
        ImportFormat::Csv => {
            let mut rdr = csv::Reader::from_path(path)
                .with_context(|| format!("failed to open CSV file {}", path.display()))?;
            rdr.deserialize()
                .collect::<std::result::Result<Vec<RawRow>, _>>()
                .context("failed to parse CSV rows (check headers match: provider,model,date,input_tokens,output_tokens,requests,cost_usd,cached_input_tokens)")?
        }
        ImportFormat::Json => {
            let raw = std::fs::read_to_string(path)
                .with_context(|| format!("failed to read JSON file {}", path.display()))?;
            serde_json::from_str(&raw)
                .context("failed to parse JSON (expected an array of usage row objects)")?
        }
    };

    if raw_rows.is_empty() {
        bail!("import file contained no rows");
    }

    let mut usage = Vec::new();
    let mut cost = Vec::new();

    for row in raw_rows {
        let provider = row
            .provider
            .clone()
            .or_else(|| provider_override.map(str::to_string))
            .context("row is missing 'provider' and no --provider override was given")?;

        let (bucket_start, bucket_end) = parse_bucket(&row.date)?;

        usage.push(UsageRecord {
            provider: provider.clone(),
            model: row.model.clone(),
            bucket_start,
            bucket_end,
            input_tokens: row.input_tokens,
            output_tokens: row.output_tokens,
            cached_input_tokens: row.cached_input_tokens,
            cache_creation_tokens: 0,
            requests: row.requests,
            cost_usd: None,
            source: "manual-import".to_string(),
        });

        if let Some(amount) = row.cost_usd {
            cost.push(CostRecord {
                provider,
                model: Some(row.model),
                bucket_start,
                bucket_end,
                amount_usd: amount,
                description: Some("manual import".to_string()),
                source: "manual-import".to_string(),
            });
        }
    }

    Ok(FetchResult { usage, cost })
}

fn parse_bucket(date: &str) -> Result<(DateTime<Utc>, DateTime<Utc>)> {
    if date.len() == 10 {
        // YYYY-MM-DD: treat as a whole UTC day.
        let start = parse_timestamp(date)?;
        return Ok((start, start + Duration::days(1)));
    }
    let start = parse_timestamp(date)?;
    Ok((start, start + Duration::hours(1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bucket_treats_bare_date_as_whole_day() {
        let (start, end) = parse_bucket("2026-05-01").unwrap();
        assert_eq!(end - start, Duration::days(1));
    }

    #[test]
    fn import_json_requires_provider_when_not_in_row_or_override() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("ai-stats-import-test-{}.json", std::process::id()));
        std::fs::write(
            &path,
            r#"[{"model":"gemini-1.5-pro","date":"2026-05-01","input_tokens":100,"output_tokens":50}]"#,
        )
        .unwrap();

        let err = import_file(&path, ImportFormat::Json, None).unwrap_err();
        assert!(err.to_string().contains("provider"));

        let ok = import_file(&path, ImportFormat::Json, Some("google")).unwrap();
        assert_eq!(ok.usage.len(), 1);
        assert_eq!(ok.usage[0].provider, "google");
        assert_eq!(ok.usage[0].input_tokens, 100);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn import_json_creates_cost_record_when_cost_present() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "ai-stats-import-cost-test-{}.json",
            std::process::id()
        ));
        std::fs::write(
            &path,
            r#"[{"provider":"xai","model":"grok-4","date":"2026-05-01","input_tokens":10,"output_tokens":5,"cost_usd":0.42}]"#,
        )
        .unwrap();

        let result = import_file(&path, ImportFormat::Json, None).unwrap();
        assert_eq!(result.cost.len(), 1);
        assert!((result.cost[0].amount_usd - 0.42).abs() < 1e-9);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn format_from_path_infers_from_extension() {
        assert_eq!(
            ImportFormat::from_path(std::path::Path::new("data.csv")).unwrap(),
            ImportFormat::Csv
        );
        assert_eq!(
            ImportFormat::from_path(std::path::Path::new("data.json")).unwrap(),
            ImportFormat::Json
        );
        assert!(ImportFormat::from_path(std::path::Path::new("data.txt")).is_err());
    }
}
