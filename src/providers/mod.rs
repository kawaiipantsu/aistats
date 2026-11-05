//! Provider integrations: each module knows how to talk to one backend's
//! real, documented API and normalize its response into [`UsageRecord`]s
//! (and, where available, [`CostRecord`]s).
//!
//! Google Gemini and xAI intentionally do not have modules here: neither
//! exposes a public, per-API-key programmatic usage/cost endpoint at the
//! time of writing. Fabricating a call against a nonexistent endpoint would
//! silently produce wrong data, so those providers are supported instead
//! through `ai-stats import` (see [`crate::import`]) using data exported
//! from their respective consoles.

pub mod anthropic;
pub mod lmstudio;
pub mod ollama;
pub mod openai;

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::models::{CostRecord, ModelInfo, UsageRecord};

/// Result of a `fetch_usage` call: normalized usage rows plus any cost rows
/// the same API call happened to produce.
#[derive(Debug, Default)]
pub struct FetchResult {
    pub usage: Vec<UsageRecord>,
    pub cost: Vec<CostRecord>,
}

#[async_trait]
pub trait UsageProvider: Send + Sync {
    /// Short machine-readable identifier, e.g. "openai".
    fn id(&self) -> &'static str;

    /// Human-readable name for CLI output, e.g. "OpenAI".
    fn display_name(&self) -> &'static str;

    /// Fetches usage (and cost, where the API provides it) for `[since,
    /// until)`. Implementations should page through the full range.
    async fn fetch_usage(&self, since: DateTime<Utc>, until: DateTime<Utc>) -> Result<FetchResult>;

    /// Performs a lightweight authenticated call to confirm credentials and
    /// connectivity, returning a short human-readable status line.
    async fn test_connection(&self) -> Result<String>;

    /// Lists models known to this provider/server, when the provider exposes
    /// model inventory (mainly useful for local servers). Hosted providers
    /// with no such endpoint may return an empty vec.
    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        Ok(Vec::new())
    }
}
