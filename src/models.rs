//! Shared domain types for normalized usage and cost data.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A normalized slice of token/request usage for one provider+model within a
/// time bucket. Providers report at varying granularity (minute/hour/day); we
/// store whatever granularity the source gave us and re-aggregate on read.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageRecord {
    pub provider: String,
    pub model: String,
    pub bucket_start: DateTime<Utc>,
    pub bucket_end: DateTime<Utc>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_input_tokens: i64,
    pub cache_creation_tokens: i64,
    pub requests: i64,
    /// Cost in USD if the provider reported it directly alongside usage.
    /// Usually `None`; cost more commonly arrives via a separate
    /// [`CostRecord`] or is estimated later from the pricing table.
    pub cost_usd: Option<f64>,
    /// Where this record came from, e.g. "openai-usage-api", "manual-import".
    pub source: String,
}

/// A normalized cost entry. Some providers (OpenAI) report cost separately
/// from token usage and not always broken down by model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostRecord {
    pub provider: String,
    pub model: Option<String>,
    pub bucket_start: DateTime<Utc>,
    pub bucket_end: DateTime<Utc>,
    pub amount_usd: f64,
    pub description: Option<String>,
    pub source: String,
}

/// A row describing a model known to a local inference server (Ollama/LM
/// Studio). These do not carry historical usage but are useful inventory
/// information for reports.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub provider: String,
    pub name: String,
    pub size_bytes: Option<i64>,
    pub parameter_size: Option<String>,
    pub quantization: Option<String>,
    pub loaded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Granularity {
    Hour,
    Day,
    Month,
}

impl Granularity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Granularity::Hour => "hour",
            Granularity::Day => "day",
            Granularity::Month => "month",
        }
    }
}

impl std::str::FromStr for Granularity {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "hour" | "hourly" | "h" => Ok(Granularity::Hour),
            "day" | "daily" | "d" => Ok(Granularity::Day),
            "month" | "monthly" | "m" => Ok(Granularity::Month),
            other => anyhow::bail!("unknown granularity '{other}' (expected hour|day|month)"),
        }
    }
}
