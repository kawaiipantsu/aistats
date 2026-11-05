//! Anthropic provider: pulls token usage and spend from the Usage & Cost
//! Admin API. Requires an **Admin API key** (`sk-ant-admin01-...`) or an
//! OAuth token with `org:admin` scope — regular API keys cannot read this
//! data.
//!
//! Reference: <https://platform.claude.com/docs/en/api/usage-cost-api>

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::models::{CostRecord, UsageRecord};

use super::{FetchResult, UsageProvider};

const ANTHROPIC_VERSION: &str = "2023-06-01";

pub struct AnthropicProvider {
    pub base_url: String,
    pub admin_api_key: String,
    client: reqwest::Client,
}

impl AnthropicProvider {
    pub fn new(base_url: String, admin_api_key: String) -> Self {
        Self {
            base_url,
            admin_api_key,
            client: reqwest::Client::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct UsageReport {
    data: Vec<UsageBucket>,
    #[serde(default)]
    has_more: bool,
    #[serde(default)]
    next_page: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UsageBucket {
    starting_at: String,
    ending_at: String,
    #[serde(default)]
    results: Vec<MessagesUsageResult>,
}

#[derive(Debug, Deserialize, Default)]
struct MessagesUsageResult {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    uncached_input_tokens: i64,
    #[serde(default)]
    cache_read_input_tokens: i64,
    #[serde(default)]
    cache_creation: Option<CacheCreation>,
    #[serde(default)]
    output_tokens: i64,
}

#[derive(Debug, Deserialize, Default)]
struct CacheCreation {
    #[serde(default)]
    ephemeral_1h_input_tokens: i64,
    #[serde(default)]
    ephemeral_5m_input_tokens: i64,
}

#[derive(Debug, Deserialize)]
struct CostReport {
    data: Vec<CostBucket>,
    #[serde(default)]
    has_more: bool,
    #[serde(default)]
    next_page: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CostBucket {
    starting_at: String,
    ending_at: String,
    #[serde(default)]
    results: Vec<CostResult>,
}

#[derive(Debug, Deserialize)]
struct CostResult {
    /// Decimal string, e.g. "123.78912" USD.
    amount: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

#[async_trait]
impl UsageProvider for AnthropicProvider {
    fn id(&self) -> &'static str {
        "anthropic"
    }

    fn display_name(&self) -> &'static str {
        "Anthropic"
    }

    async fn test_connection(&self) -> Result<String> {
        let url = format!("{}/organizations/usage_report/messages", self.base_url);
        let resp = self
            .client
            .get(&url)
            .header("x-api-key", &self.admin_api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .query(&[("limit", "1")])
            .send()
            .await
            .context("request to Anthropic usage API failed")?;
        let status = resp.status();
        if status.is_success() {
            Ok("OK: admin key accepted by the Usage & Cost Admin API".to_string())
        } else {
            let body = resp.text().await.unwrap_or_default();
            bail!("Anthropic usage API returned HTTP {status}: {body}")
        }
    }

    async fn fetch_usage(&self, since: DateTime<Utc>, until: DateTime<Utc>) -> Result<FetchResult> {
        let mut usage = Vec::new();
        let mut cost = Vec::new();

        // --- Token usage ---
        let mut page: Option<String> = None;
        loop {
            let url = format!("{}/organizations/usage_report/messages", self.base_url);
            let mut req = self
                .client
                .get(&url)
                .header("x-api-key", &self.admin_api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .query(&[
                    ("starting_at", since.to_rfc3339()),
                    ("ending_at", until.to_rfc3339()),
                    ("bucket_width", "1h".to_string()),
                    ("limit", "168".to_string()),
                ])
                .query(&[("group_by[]", "model")]);
            if let Some(p) = &page {
                req = req.query(&[("page", p.clone())]);
            }
            let resp = req.send().await.context("Anthropic usage request failed")?;
            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                bail!("Anthropic usage API returned HTTP {status}: {body}");
            }
            let parsed: UsageReport = resp
                .json()
                .await
                .context("failed to parse Anthropic usage response")?;

            for bucket in &parsed.data {
                let bucket_start = DateTime::parse_from_rfc3339(&bucket.starting_at)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or(since);
                let bucket_end = DateTime::parse_from_rfc3339(&bucket.ending_at)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or(until);
                for r in &bucket.results {
                    let cache_creation = r
                        .cache_creation
                        .as_ref()
                        .map(|c| c.ephemeral_1h_input_tokens + c.ephemeral_5m_input_tokens)
                        .unwrap_or(0);
                    usage.push(UsageRecord {
                        provider: "anthropic".to_string(),
                        model: r.model.clone().unwrap_or_else(|| "unknown".to_string()),
                        bucket_start,
                        bucket_end,
                        input_tokens: r.uncached_input_tokens + r.cache_read_input_tokens,
                        output_tokens: r.output_tokens,
                        cached_input_tokens: r.cache_read_input_tokens,
                        cache_creation_tokens: cache_creation,
                        requests: 0, // not reported per-bucket by this endpoint
                        cost_usd: None,
                        source: "anthropic-usage-api".to_string(),
                    });
                }
            }

            if parsed.has_more {
                page = parsed.next_page;
                if page.is_none() {
                    break;
                }
            } else {
                break;
            }
        }

        // --- Cost (grouped by description, which includes the model name) ---
        let mut page: Option<String> = None;
        loop {
            let url = format!("{}/organizations/cost_report", self.base_url);
            let mut req = self
                .client
                .get(&url)
                .header("x-api-key", &self.admin_api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .query(&[
                    ("starting_at", since.to_rfc3339()),
                    ("ending_at", until.to_rfc3339()),
                    ("limit", "31".to_string()),
                ])
                .query(&[("group_by[]", "description")]);
            if let Some(p) = &page {
                req = req.query(&[("page", p.clone())]);
            }
            let resp = req.send().await.context("Anthropic cost request failed")?;
            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                bail!("Anthropic cost API returned HTTP {status}: {body}");
            }
            let parsed: CostReport = resp
                .json()
                .await
                .context("failed to parse Anthropic cost response")?;

            for bucket in &parsed.data {
                let bucket_start = DateTime::parse_from_rfc3339(&bucket.starting_at)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or(since);
                let bucket_end = DateTime::parse_from_rfc3339(&bucket.ending_at)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or(until);
                for r in &bucket.results {
                    // The API documents `amount` as a decimal string in the
                    // lowest currency unit (cents for USD), e.g. "123.45"
                    // represents $1.23.
                    let amount: f64 = r.amount.parse::<f64>().unwrap_or(0.0) / 100.0;
                    cost.push(CostRecord {
                        provider: "anthropic".to_string(),
                        model: r.model.clone(),
                        bucket_start,
                        bucket_end,
                        amount_usd: amount,
                        description: r.description.clone(),
                        source: "anthropic-cost-api".to_string(),
                    });
                }
            }

            if parsed.has_more {
                page = parsed.next_page;
                if page.is_none() {
                    break;
                }
            } else {
                break;
            }
        }

        Ok(FetchResult { usage, cost })
    }
}
