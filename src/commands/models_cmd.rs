use anyhow::Result;

use crate::config::Config;

use super::{enabled_providers, find_provider, require_known_provider};

pub async fn run(cfg: &Config, provider: Option<&str>) -> Result<()> {
    let targets = if let Some(id) = provider {
        require_known_provider(id)?;
        vec![find_provider(cfg, id)?]
    } else {
        enabled_providers(cfg)
    };

    let mut any = false;
    for p in targets {
        match p.list_models().await {
            Ok(models) if !models.is_empty() => {
                any = true;
                println!("{} ({})", p.display_name(), models.len());
                println!(
                    "{:<40} {:<8} {:<10} {:<12} SIZE",
                    "MODEL", "LOADED", "PARAMS", "QUANT"
                );
                for m in models {
                    let size = m
                        .size_bytes
                        .map(|b| format!("{:.1} GB", b as f64 / 1_000_000_000.0))
                        .unwrap_or_default();
                    println!(
                        "{:<40} {:<8} {:<10} {:<12} {}",
                        m.name,
                        if m.loaded { "yes" } else { "no" },
                        m.parameter_size.unwrap_or_default(),
                        m.quantization.unwrap_or_default(),
                        size
                    );
                }
                println!();
            }
            Ok(_) => {}
            Err(e) => eprintln!("{}: {e:#}", p.display_name()),
        }
    }

    if !any {
        println!(
            "No local model inventory available (Ollama/LM Studio not reachable or not enabled)."
        );
    }
    Ok(())
}
