use anyhow::Result;

use crate::config::Config;
use crate::{db, paths};

pub fn run() -> Result<()> {
    paths::ensure_dirs()?;

    let config_path = paths::config_file()?;
    if config_path.exists() {
        println!("Config already exists at {}", config_path.display());
    } else {
        Config::default().save(&config_path)?;
        println!("Created config at {}", config_path.display());
    }

    let db_path = paths::database_file()?;
    let _ = db::open(&db_path)?;
    println!("Initialized usage database at {}", db_path.display());

    println!();
    println!("Next steps:");
    println!(
        "  1. Edit {} to enable providers and set admin API keys,",
        config_path.display()
    );
    println!("     or set OPENAI_ADMIN_API_KEY / ANTHROPIC_ADMIN_API_KEY in your environment.");
    println!("  2. Run `ai-stats providers test` to verify connectivity.");
    println!("  3. Run `ai-stats sync` to pull usage data.");
    println!("  4. Run `ai-stats report` to see a summary.");

    Ok(())
}
