//! LM Studio provider: local inference server inventory.
//!
//! Like Ollama, LM Studio's local server does not keep historical per-request
//! token usage, so `fetch_usage` is always empty. Model inventory comes from
//! the OpenAI-compatible `GET /v1/models` endpoint, which every LM Studio
//! server version supports.

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::models::ModelInfo;

use super::{FetchResult, UsageProvider};

pub struct LmStudioProvider {
    pub base_url: String,
    client: reqwest::Client,
}

impl LmStudioProvider {
    pub fn new(base_url: String) -> Self {
        Self {
            base_url,
            client: reqwest::Client::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    #[serde(default)]
    data: Vec<ModelEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
    id: String,
}

#[async_trait]
impl UsageProvider for LmStudioProvider {
    fn id(&self) -> &'static str {
        "lmstudio"
    }

    fn display_name(&self) -> &'static str {
        "LM Studio (local)"
    }

    async fn test_connection(&self) -> Result<String> {
        let url = format!("{}/v1/models", self.base_url);
        let resp = self.client.get(&url).send().await.with_context(|| {
            format!(
                "could not reach LM Studio at {} (is the local server running?)",
                self.base_url
            )
        })?;
        if resp.status().is_success() {
            Ok("OK: LM Studio server reachable".to_string())
        } else {
            bail!("LM Studio returned HTTP {}", resp.status())
        }
    }

    async fn fetch_usage(
        &self,
        _since: DateTime<Utc>,
        _until: DateTime<Utc>,
    ) -> Result<FetchResult> {
        // LM Studio does not keep historical usage; nothing to ingest here.
        Ok(FetchResult::default())
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let url = format!("{}/v1/models", self.base_url);
        let resp: ModelsResponse = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("could not reach LM Studio at {}", self.base_url))?
            .json()
            .await
            .context("failed to parse LM Studio /v1/models response")?;

        Ok(resp
            .data
            .into_iter()
            .map(|m| ModelInfo {
                provider: "lmstudio".to_string(),
                name: m.id,
                size_bytes: None,
                parameter_size: None,
                quantization: None,
                loaded: true,
            })
            .collect())
    }
}
