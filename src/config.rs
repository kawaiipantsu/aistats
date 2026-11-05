//! Configuration file schema, loading, saving, and secret resolution.
//!
//! The config file (`config.yaml`) is the single place a user edits to enable
//! providers and store credentials. API keys can also be supplied via
//! environment variables, which always take priority over the file so users
//! can keep secrets out of disk entirely if they prefer.

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths;

fn default_true() -> bool {
    true
}

fn default_false() -> bool {
    false
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub general: GeneralConfig,
    #[serde(default)]
    pub providers: ProvidersConfig,
    #[serde(default)]
    pub pricing: PricingConfig,
    #[serde(default)]
    pub discord: DiscordConfig,
}

fn default_version() -> u32 {
    1
}

impl Default for Config {
    fn default() -> Self {
        Config {
            version: default_version(),
            general: GeneralConfig::default(),
            providers: ProvidersConfig::default(),
            pricing: PricingConfig::default(),
            discord: DiscordConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    /// IANA timezone name (e.g. "Europe/Copenhagen") or "UTC"/"local".
    /// Used only for display; all storage and bucketing is UTC internally.
    #[serde(default = "default_timezone")]
    pub timezone: String,
    #[serde(default = "default_currency")]
    pub currency: String,
}

fn default_timezone() -> String {
    "UTC".to_string()
}

fn default_currency() -> String {
    "USD".to_string()
}

impl Default for GeneralConfig {
    fn default() -> Self {
        GeneralConfig {
            timezone: default_timezone(),
            currency: default_currency(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProvidersConfig {
    #[serde(default = "openai_default")]
    pub openai: OpenAiConfig,
    #[serde(default = "anthropic_default")]
    pub anthropic: AnthropicConfig,
    #[serde(default = "ollama_default")]
    pub ollama: OllamaConfig,
    #[serde(default = "lmstudio_default")]
    pub lmstudio: LmStudioConfig,
}

fn openai_default() -> OpenAiConfig {
    OpenAiConfig::default()
}
fn anthropic_default() -> AnthropicConfig {
    AnthropicConfig::default()
}
fn ollama_default() -> OllamaConfig {
    OllamaConfig::default()
}
fn lmstudio_default() -> LmStudioConfig {
    LmStudioConfig::default()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiConfig {
    #[serde(default = "default_false")]
    pub enabled: bool,
    /// Organization admin API key (starts with `sk-admin-...`). Required for
    /// the Usage & Costs endpoints. Prefer the OPENAI_ADMIN_API_KEY env var.
    #[serde(default)]
    pub admin_api_key: Option<String>,
    #[serde(default = "openai_base_url")]
    pub base_url: String,
}

fn openai_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}

impl Default for OpenAiConfig {
    fn default() -> Self {
        OpenAiConfig {
            enabled: false,
            admin_api_key: None,
            base_url: openai_base_url(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicConfig {
    #[serde(default = "default_false")]
    pub enabled: bool,
    /// Admin API key (starts with `sk-ant-admin...`). Prefer the
    /// ANTHROPIC_ADMIN_API_KEY env var.
    #[serde(default)]
    pub admin_api_key: Option<String>,
    #[serde(default = "anthropic_base_url")]
    pub base_url: String,
}

fn anthropic_base_url() -> String {
    "https://api.anthropic.com/v1".to_string()
}

impl Default for AnthropicConfig {
    fn default() -> Self {
        AnthropicConfig {
            enabled: false,
            admin_api_key: None,
            base_url: anthropic_base_url(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "ollama_base_url")]
    pub base_url: String,
}

fn ollama_base_url() -> String {
    "http://localhost:11434".to_string()
}

impl Default for OllamaConfig {
    fn default() -> Self {
        OllamaConfig {
            enabled: true,
            base_url: ollama_base_url(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LmStudioConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "lmstudio_base_url")]
    pub base_url: String,
}

fn lmstudio_base_url() -> String {
    "http://localhost:1234".to_string()
}

impl Default for LmStudioConfig {
    fn default() -> Self {
        LmStudioConfig {
            enabled: true,
            base_url: lmstudio_base_url(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PricingConfig {
    /// Optional per-model price overrides, in USD per 1,000,000 tokens.
    /// Keys are model names as reported by the provider. These override the
    /// built-in pricing table used to estimate cost when a provider does not
    /// return cost data directly.
    #[serde(default)]
    pub overrides: std::collections::BTreeMap<String, ModelPriceOverride>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPriceOverride {
    pub input_per_million: f64,
    pub output_per_million: f64,
    #[serde(default)]
    pub cached_input_per_million: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DiscordConfig {
    #[serde(default)]
    pub webhook_url: Option<String>,
}

impl Config {
    pub fn load_or_default(path: &Path) -> Result<Config> {
        if !path.exists() {
            return Ok(Config::default());
        }
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config file {}", path.display()))?;
        let cfg: Config = serde_yaml::from_str(&raw)
            .with_context(|| format!("failed to parse config file {}", path.display()))?;
        Ok(cfg)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let yaml = serde_yaml::to_string(self).context("failed to serialize config")?;
        let header = "# ai-stats configuration\n\
             # Secrets stored here are protected with 0600 permissions, but\n\
             # environment variables always take priority when set:\n\
             #   OPENAI_ADMIN_API_KEY, ANTHROPIC_ADMIN_API_KEY\n\
             # See docs: https://platform.openai.com/docs/api-reference/usage\n\
             #           https://platform.claude.com/docs/en/api/usage-cost-api\n\n";
        std::fs::write(path, format!("{header}{yaml}"))
            .with_context(|| format!("failed to write config file {}", path.display()))?;
        paths::restrict_permissions(path)?;
        Ok(())
    }
}

/// Resolves the effective OpenAI admin key: env var wins over config file.
pub fn resolve_openai_admin_key(cfg: &Config) -> Option<String> {
    std::env::var("OPENAI_ADMIN_API_KEY")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| cfg.providers.openai.admin_api_key.clone())
}

/// Resolves the effective Anthropic admin key: env var wins over config file.
pub fn resolve_anthropic_admin_key(cfg: &Config) -> Option<String> {
    std::env::var("ANTHROPIC_ADMIN_API_KEY")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| cfg.providers.anthropic.admin_api_key.clone())
}

/// Resolves the effective Discord webhook URL: env var wins over config file.
pub fn resolve_discord_webhook(cfg: &Config) -> Option<String> {
    std::env::var("AI_STATS_DISCORD_WEBHOOK_URL")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| cfg.discord.webhook_url.clone())
}

/// Renders a copy of the config with secrets masked, for `config show`.
pub fn masked(cfg: &Config) -> Config {
    let mut c = cfg.clone();
    let mask = |s: &Option<String>| s.as_ref().map(|v| mask_secret(v));
    c.providers.openai.admin_api_key = mask(&c.providers.openai.admin_api_key);
    c.providers.anthropic.admin_api_key = mask(&c.providers.anthropic.admin_api_key);
    c.discord.webhook_url = c.discord.webhook_url.map(|_| "••••••••".to_string());
    c
}

fn mask_secret(s: &str) -> String {
    if s.len() <= 8 {
        return "••••••••".to_string();
    }
    format!("{}••••••••{}", &s[..4], &s[s.len() - 4..])
}
