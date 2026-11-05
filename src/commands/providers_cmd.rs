use anyhow::Result;

use crate::config::Config;

use super::{enabled_providers, find_provider, require_known_provider};

pub fn list(cfg: &Config) {
    println!("{:<12} {:<9} BASE URL / NOTES", "PROVIDER", "ENABLED");
    println!("{}", "-".repeat(60));
    println!(
        "{:<12} {:<9} {}",
        "openai", cfg.providers.openai.enabled, cfg.providers.openai.base_url
    );
    println!(
        "{:<12} {:<9} {}",
        "anthropic", cfg.providers.anthropic.enabled, cfg.providers.anthropic.base_url
    );
    println!(
        "{:<12} {:<9} {}",
        "ollama", cfg.providers.ollama.enabled, cfg.providers.ollama.base_url
    );
    println!(
        "{:<12} {:<9} {}",
        "lmstudio", cfg.providers.lmstudio.enabled, cfg.providers.lmstudio.base_url
    );
    println!(
        "{:<12} {:<9} no public usage API; use `ai-stats import`",
        "google", false
    );
    println!(
        "{:<12} {:<9} no public usage API; use `ai-stats import`",
        "xai", false
    );
}

pub async fn test(cfg: &Config, provider: Option<&str>) -> Result<()> {
    let targets: Vec<Box<dyn crate::providers::UsageProvider>> = if let Some(id) = provider {
        require_known_provider(id)?;
        vec![find_provider(cfg, id)?]
    } else {
        enabled_providers(cfg)
    };

    if targets.is_empty() {
        println!("No providers are enabled with valid credentials. See `ai-stats config show`.");
        return Ok(());
    }

    for p in targets {
        print!("{:<20} ", p.display_name());
        match p.test_connection().await {
            Ok(msg) => println!("{msg}"),
            Err(e) => println!("FAILED: {e}"),
        }
    }
    Ok(())
}
