pub mod config_cmd;
pub mod fun_cmd;
pub mod import_cmd;
pub mod init;
pub mod models_cmd;
pub mod providers_cmd;
pub mod report_cmd;
pub mod retention_cmd;
pub mod sync_cmd;

use anyhow::{Result, bail};

use crate::config::Config;
use crate::providers::UsageProvider;
use crate::providers::anthropic::AnthropicProvider;
use crate::providers::lmstudio::LmStudioProvider;
use crate::providers::ollama::OllamaProvider;
use crate::providers::openai::OpenAiProvider;

/// Builds the enabled, credential-complete providers from config. Providers
/// that are enabled but missing required credentials are skipped with a
/// warning rather than causing the whole command to fail, so `sync`/`report`
/// still work for whichever providers *are* ready.
pub fn enabled_providers(cfg: &Config) -> Vec<Box<dyn UsageProvider>> {
    let mut providers: Vec<Box<dyn UsageProvider>> = Vec::new();

    if cfg.providers.openai.enabled {
        match crate::config::resolve_openai_admin_key(cfg) {
            Some(key) if !key.is_empty() => providers.push(Box::new(OpenAiProvider::new(
                cfg.providers.openai.base_url.clone(),
                key,
            ))),
            _ => eprintln!(
                "warning: OpenAI is enabled but no admin API key is set (OPENAI_ADMIN_API_KEY or providers.openai.admin_api_key) — skipping"
            ),
        }
    }
    if cfg.providers.anthropic.enabled {
        match crate::config::resolve_anthropic_admin_key(cfg) {
            Some(key) if !key.is_empty() => providers.push(Box::new(AnthropicProvider::new(
                cfg.providers.anthropic.base_url.clone(),
                key,
            ))),
            _ => eprintln!(
                "warning: Anthropic is enabled but no admin API key is set (ANTHROPIC_ADMIN_API_KEY or providers.anthropic.admin_api_key) — skipping"
            ),
        }
    }
    if cfg.providers.ollama.enabled {
        providers.push(Box::new(OllamaProvider::new(
            cfg.providers.ollama.base_url.clone(),
        )));
    }
    if cfg.providers.lmstudio.enabled {
        providers.push(Box::new(LmStudioProvider::new(
            cfg.providers.lmstudio.base_url.clone(),
        )));
    }

    providers
}

pub fn find_provider(cfg: &Config, id: &str) -> Result<Box<dyn UsageProvider>> {
    enabled_providers(cfg)
        .into_iter()
        .find(|p| p.id() == id)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "provider '{id}' is not enabled or is missing credentials (run `ai-stats providers list`)"
            )
        })
}

pub fn known_provider_ids() -> &'static [&'static str] {
    &["openai", "anthropic", "ollama", "lmstudio"]
}

pub fn require_known_provider(id: &str) -> Result<()> {
    if known_provider_ids().contains(&id) {
        Ok(())
    } else {
        bail!(
            "unknown provider '{id}' (known: {})",
            known_provider_ids().join(", ")
        )
    }
}
