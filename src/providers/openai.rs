//! OpenAI provider: pulls token usage from the Completions Usage API and
//! spend from the Costs API. Both require an **organization admin key**
//! (`sk-admin-...`), not a regular project API key — OpenAI does not expose
//! historical usage/cost data to normal API keys.
//!
//! Reference: <https://platform.openai.com/docs/api-reference/usage>
//!
//! Known limitation: only chat/completions usage is pulled. OpenAI also
//! reports separate usage buckets for embeddings, images, audio, moderations
//! and vector stores; those are not yet ingested by this tool.

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::models::{CostRecord, UsageRecord};

use super::{FetchResult, UsageProvider};

pub struct OpenAiProvider {
    pub base_url: String,
    pub admin_api_key: String,
    client: reqwest::Client,
}

impl OpenAiProvider {
    pub fn new(base_url: String, admin_api_key: String) -> Self {
        Self {
            base_url,
            admin_api_key,
            client: reqwest::Client::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct UsagePage {
    data: Vec<UsageBucket>,
    #[serde(default)]
    has_more: bool,
    #[serde(default)]
    next_page: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UsageBucket {
    start_time: i64,
    end_time: i64,
    #[serde(default)]
    results: Vec<CompletionsResult>,
}

#[derive(Debug, Deserialize, Default)]
struct CompletionsResult {
    #[serde(default)]
    input_tokens: i64,
    #[serde(default)]
    output_tokens: i64,
    #[serde(default)]
    input_cached_tokens: i64,
    #[serde(default)]
    num_model_requests: i64,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CostsPage {
    data: Vec<CostBucket>,
    #[serde(default)]
    has_more: bool,
    #[serde(default)]
    next_page: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CostBucket {
    start_time: i64,
    end_time: i64,
    #[serde(default)]
    results: Vec<CostResult>,
}

#[derive(Debug, Deserialize)]
struct CostResult {
    amount: CostAmount,
    #[serde(default)]
    line_item: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CostAmount {
    value: f64,
}

#[async_trait]
impl UsageProvider for OpenAiProvider {
    fn id(&self) -> &'static str {
        "openai"
    }

    fn display_name(&self) -> &'static str {
        "OpenAI"
    }

    async fn test_connection(&self) -> Result<String> {
        let url = format!("{}/organization/usage/completions", self.base_url);
        let resp = self
            .client
            .get(&url)
            .bearer_auth(&self.admin_api_key)
            .query(&[("start_time", (Utc::now().timestamp() - 3600).to_string())])
            .query(&[("limit", "1")])
            .send()
            .await
            .context("request to OpenAI usage API failed")?;
        let status = resp.status();
        if status.is_success() {
            Ok("OK: admin key accepted by the Usage API".to_string())
        } else {
            let body = resp.text().await.unwrap_or_default();
            bail!("OpenAI usage API returned HTTP {status}: {body}")
        }
    }

    async fn fetch_usage(&self, since: DateTime<Utc>, until: DateTime<Utc>) -> Result<FetchResult> {
        let mut usage = Vec::new();
        let mut cost = Vec::new();

        // --- Token usage (completions) ---
        let mut page: Option<String> = None;
        loop {
            let url = format!("{}/organization/usage/completions", self.base_url);
            let mut req = self
                .client
                .get(&url)
                .bearer_auth(&self.admin_api_key)
                .query(&[
                    ("start_time", since.timestamp().to_string()),
                    ("end_time", until.timestamp().to_string()),
                    ("bucket_width", "1h".to_string()),
                    ("limit", "168".to_string()),
                ])
                .query(&[("group_by[]", "model")]);
            if let Some(p) = &page {
                req = req.query(&[("page", p.clone())]);
            }
            let resp = req.send().await.context("OpenAI usage request failed")?;
            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                bail!("OpenAI usage API returned HTTP {status}: {body}");
            }
            let parsed: UsagePage = resp
                .json()
                .await
                .context("failed to parse OpenAI usage response")?;

            for bucket in &parsed.data {
                let bucket_start = DateTime::from_timestamp(bucket.start_time, 0).unwrap_or(since);
                let bucket_end = DateTime::from_timestamp(bucket.end_time, 0).unwrap_or(until);
                for r in &bucket.results {
                    usage.push(UsageRecord {
                        provider: "openai".to_string(),
                        model: r.model.clone().unwrap_or_else(|| "unknown".to_string()),
                        bucket_start,
                        bucket_end,
                        input_tokens: r.input_tokens,
                        output_tokens: r.output_tokens,
                        cached_input_tokens: r.input_cached_tokens,
                        cache_creation_tokens: 0,
                        requests: r.num_model_requests,
                        cost_usd: None,
                        source: "openai-usage-api".to_string(),
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

        // --- Cost (daily, not attributable to a specific model) ---
        let mut page: Option<String> = None;
        loop {
            let url = format!("{}/organization/costs", self.base_url);
            let mut req = self
                .client
                .get(&url)
                .bearer_auth(&self.admin_api_key)
                .query(&[
                    ("start_time", since.timestamp().to_string()),
                    ("end_time", until.timestamp().to_string()),
                    ("limit", "31".to_string()),
                ]);
            if let Some(p) = &page {
                req = req.query(&[("page", p.clone())]);
            }
            let resp = req.send().await.context("OpenAI costs request failed")?;
            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                bail!("OpenAI costs API returned HTTP {status}: {body}");
            }
            let parsed: CostsPage = resp
                .json()
                .await
                .context("failed to parse OpenAI costs response")?;

            for bucket in &parsed.data {
                let bucket_start = DateTime::from_timestamp(bucket.start_time, 0).unwrap_or(since);
                let bucket_end = DateTime::from_timestamp(bucket.end_time, 0).unwrap_or(until);
                for r in &bucket.results {
                    cost.push(CostRecord {
                        provider: "openai".to_string(),
                        model: None,
                        bucket_start,
                        bucket_end,
                        amount_usd: r.amount.value,
                        description: r.line_item.clone(),
                        source: "openai-costs-api".to_string(),
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
