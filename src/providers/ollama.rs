//! Ollama provider: local inference server inventory.
//!
//! Ollama does not persist historical per-request token usage anywhere the
//! API exposes, so `fetch_usage` always returns empty — there is nothing to
//! sync. What we *can* do honestly is report the local model catalog and
//! which models are currently loaded into memory, which is useful inventory
//! for a report even without cost/token history.
//!
//! Endpoints: `GET /api/tags` (installed models), `GET /api/ps` (loaded
//! models). Both are part of Ollama's stable local REST API.

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::models::ModelInfo;

use super::{FetchResult, UsageProvider};

pub struct OllamaProvider {
    pub base_url: String,
    client: reqwest::Client,
}

impl OllamaProvider {
    pub fn new(base_url: String) -> Self {
        Self {
            base_url,
            client: reqwest::Client::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct TagsResponse {
    #[serde(default)]
    models: Vec<TagModel>,
}

#[derive(Debug, Deserialize)]
struct TagModel {
    name: String,
    #[serde(default)]
    size: Option<i64>,
    #[serde(default)]
    details: Option<ModelDetails>,
}

#[derive(Debug, Deserialize)]
struct ModelDetails {
    #[serde(default)]
    parameter_size: Option<String>,
    #[serde(default)]
    quantization_level: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PsResponse {
    #[serde(default)]
    models: Vec<PsModel>,
}

#[derive(Debug, Deserialize)]
struct PsModel {
    name: String,
}

#[async_trait]
impl UsageProvider for OllamaProvider {
    fn id(&self) -> &'static str {
        "ollama"
    }

    fn display_name(&self) -> &'static str {
        "Ollama (local)"
    }

    async fn test_connection(&self) -> Result<String> {
        let url = format!("{}/api/tags", self.base_url);
        let resp = self.client.get(&url).send().await.with_context(|| {
            format!(
                "could not reach Ollama at {} (is it running?)",
                self.base_url
            )
        })?;
        if resp.status().is_success() {
            Ok("OK: Ollama server reachable".to_string())
        } else {
            bail!("Ollama returned HTTP {}", resp.status())
        }
    }

    async fn fetch_usage(
        &self,
        _since: DateTime<Utc>,
        _until: DateTime<Utc>,
    ) -> Result<FetchResult> {
        // Ollama does not keep historical usage; nothing to ingest here.
        // See `list_models` for the inventory this provider does offer.
        Ok(FetchResult::default())
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let tags_url = format!("{}/api/tags", self.base_url);
        let tags: TagsResponse = self
            .client
            .get(&tags_url)
            .send()
            .await
            .with_context(|| format!("could not reach Ollama at {}", self.base_url))?
            .json()
            .await
            .context("failed to parse Ollama /api/tags response")?;

        let loaded_names: std::collections::HashSet<String> = {
            let ps_url = format!("{}/api/ps", self.base_url);
            match self.client.get(&ps_url).send().await {
                Ok(resp) if resp.status().is_success() => resp
                    .json::<PsResponse>()
                    .await
                    .map(|p| p.models.into_iter().map(|m| m.name).collect())
                    .unwrap_or_default(),
                _ => Default::default(),
            }
        };

        Ok(tags
            .models
            .into_iter()
            .map(|m| ModelInfo {
                provider: "ollama".to_string(),
                loaded: loaded_names.contains(&m.name),
                size_bytes: m.size,
                parameter_size: m.details.as_ref().and_then(|d| d.parameter_size.clone()),
                quantization: m.details.and_then(|d| d.quantization_level),
                name: m.name,
            })
            .collect())
    }
}
