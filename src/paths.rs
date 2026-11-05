//! Resolves the ai-stats config directory and the well-known locations inside it.
//!
//! Everything the CLI owns lives under a single directory, honoring the user's
//! explicit request for `.config/ai-stats/`: config, cache database, sync
//! cursors, logs, and default report output all nest under this root so the
//! tool is easy to back up, inspect, or wipe.

use std::path::PathBuf;

use anyhow::{Context, Result};

/// Environment variable that overrides the config directory (mainly for tests
/// and advanced setups where the default location is not desired).
const ENV_OVERRIDE: &str = "AI_STATS_CONFIG_DIR";

/// Returns the root config directory, e.g. `~/.config/ai-stats`.
///
/// Resolution order:
/// 1. `AI_STATS_CONFIG_DIR` environment variable, if set.
/// 2. `$XDG_CONFIG_HOME/ai-stats`, if `XDG_CONFIG_HOME` is set.
/// 3. `$HOME/.config/ai-stats` (or `%USERPROFILE%\.config\ai-stats` on Windows).
///
/// This deliberately does not use platform-specific conventions (e.g. macOS
/// `Library/Application Support`) because the user explicitly requested a
/// `.config/ai-stats/` layout everywhere.
pub fn config_dir() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var(ENV_OVERRIDE) {
        return Ok(PathBuf::from(dir));
    }

    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME")
        && !xdg.is_empty()
    {
        return Ok(PathBuf::from(xdg).join("ai-stats"));
    }

    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .context("could not determine home directory (HOME/USERPROFILE not set)")?;

    Ok(PathBuf::from(home).join(".config").join("ai-stats"))
}

pub fn config_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.yaml"))
}

pub fn cache_dir() -> Result<PathBuf> {
    Ok(config_dir()?.join("cache"))
}

pub fn database_file() -> Result<PathBuf> {
    Ok(cache_dir()?.join("usage.db"))
}

pub fn logs_dir() -> Result<PathBuf> {
    Ok(config_dir()?.join("logs"))
}

pub fn reports_dir() -> Result<PathBuf> {
    Ok(config_dir()?.join("reports"))
}

/// Creates the full directory skeleton under the config root. Safe to call
/// repeatedly.
pub fn ensure_dirs() -> Result<()> {
    for dir in [config_dir()?, cache_dir()?, logs_dir()?, reports_dir()?] {
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create directory {}", dir.display()))?;
    }
    Ok(())
}

/// Restricts a file to owner-only read/write (0600) on Unix. No-op elsewhere.
pub fn restrict_permissions(path: &std::path::Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(path, perms)
            .with_context(|| format!("failed to set permissions on {}", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}
