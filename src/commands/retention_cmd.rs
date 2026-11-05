//! `--add-retention-expansion`: changes the AI CLI settings that decide how
//! long chat logs are kept, after showing the plan and getting a typed "yes".
//!
//! Claude Code deletes transcripts older than `cleanupPeriodDays` at startup.
//! Codex has no session cleanup setting, so its only levers are the `[history]`
//! options, which limit `history.jsonl`.
//!
//! This edits only the caller's own settings. Writing to other users' homes as
//! root would mean following paths those users control.

use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde_json::{Map, Value};

use crate::fun::discover;
use crate::report::fun::fmt_compact;

const CLAUDE_KEY: &str = "cleanupPeriodDays";
const CLAUDE_DEFAULT_DAYS: u64 = 30;

/// What a settings file needs, worked out from its current text.
#[derive(Debug, PartialEq, Eq)]
enum Edit {
    Nothing(String),
    Manual(String),
    Change { summary: String, new_text: String },
}

#[derive(Debug, Clone, Copy)]
enum Tool {
    Claude,
    Codex,
}

struct Target {
    tool: Tool,
    name: &'static str,
    path: PathBuf,
}

fn plan(target: &Target, days: u64) -> Result<Edit> {
    let text = match fs::read_to_string(&target.path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => {
            return Err(e).with_context(|| format!("failed to read {}", target.path.display()));
        }
    };
    match target.tool {
        Tool::Claude => claude_edit(text.as_deref(), days)
            .with_context(|| format!("cannot update {}", target.path.display())),
        Tool::Codex => Ok(codex_edit(text.as_deref())),
    }
}

/// Sets `cleanupPeriodDays`, never lowering it. Key order and everything else
/// in the file is preserved. Zero is not an option: Claude Code rejects it,
/// and a rejected value pauses cleanup with a settings error at every start.
fn claude_edit(text: Option<&str>, days: u64) -> Result<Edit> {
    let mut settings: Map<String, Value> = match text {
        Some(text) if !text.trim().is_empty() => serde_json::from_str(text)
            .context("it is not a valid JSON object, so it was left alone")?,
        _ => Map::new(),
    };

    let current = settings.get(CLAUDE_KEY).and_then(Value::as_u64);
    if current.is_some_and(|value| value >= days) {
        return Ok(Edit::Nothing(format!(
            "{CLAUDE_KEY} is already {} days",
            current.unwrap_or(days)
        )));
    }

    let from = match current {
        Some(value) => format!("{value} days"),
        None => format!("not set (default {CLAUDE_DEFAULT_DAYS} days)"),
    };
    settings.insert(CLAUDE_KEY.to_string(), Value::from(days));

    let mut new_text = serde_json::to_string_pretty(&settings)?;
    if text.is_none_or(|text| text.ends_with('\n')) {
        new_text.push('\n');
    }
    Ok(Edit::Change {
        summary: format!("{CLAUDE_KEY}: {from} -> {days} days"),
        new_text,
    })
}

/// Splits a TOML line into its key and value, ignoring comments and headers.
fn toml_key(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    if line.starts_with(['#', '[']) {
        return None;
    }
    let (key, value) = line.split_once('=')?;
    Some((key.trim(), value.trim()))
}

/// Removes `[history]` limits from a Codex config. There is no TOML library in
/// this project, so this edits lines and only recognises the plain forms; an
/// inline table is left for the user rather than risk mangling the file.
fn codex_edit(text: Option<&str>) -> Edit {
    let Some(text) = text else {
        return Edit::Nothing("no limits configured".to_string());
    };

    let mut table = String::new();
    let mut output = String::with_capacity(text.len());
    let mut changes = Vec::new();

    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            table = trimmed.trim_matches(['[', ']']).trim().to_string();
        }
        let key = toml_key(line).map(|(key, value)| {
            let full = if table.is_empty() {
                key.to_string()
            } else {
                format!("{table}.{key}")
            };
            (full, value)
        });
        match key.as_ref().map(|(key, value)| (key.as_str(), *value)) {
            Some(("history", _)) => {
                return Edit::Manual(
                    "[history] is an inline table: remove any persistence = \"none\" or \
                     max_bytes from it by hand"
                        .to_string(),
                );
            }
            Some(("history.persistence", value)) if value.starts_with("\"none\"") => {
                let indent = &line[..line.len() - line.trim_start().len()];
                let prefix = if table.is_empty() { "history." } else { "" };
                let ending = if line.ends_with('\n') { "\n" } else { "" };
                output.push_str(&format!(
                    "{indent}{prefix}persistence = \"save-all\"{ending}"
                ));
                changes.push("history.persistence: \"none\" -> \"save-all\"");
            }
            Some(("history.max_bytes", _)) => {
                output.push_str("# ");
                output.push_str(line);
                changes.push("history.max_bytes: limit removed");
            }
            _ => output.push_str(line),
        }
    }

    if changes.is_empty() {
        Edit::Nothing("no limits configured".to_string())
    } else {
        Edit::Change {
            summary: changes.join(", "),
            new_text: output,
        }
    }
}

/// Only exactly "yes" proceeds. Anything else, including end of input, cancels.
fn confirmed(input: &mut impl BufRead) -> Result<bool> {
    let mut answer = String::new();
    input.read_line(&mut answer)?;
    Ok(answer.trim() == "yes")
}

/// Replaces a file by writing beside it and renaming, so a crash cannot leave
/// a half-written settings file. The original is copied to a timestamped
/// backup first. A symlinked settings file is written through, which keeps
/// dotfile managers working.
fn apply(path: &Path, new_text: &str) -> Result<Option<PathBuf>> {
    let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let name = path
        .file_name()
        .context("settings path has no file name")?
        .to_string_lossy();

    let mut backup = None;
    let mode = match fs::metadata(&path) {
        Ok(meta) => {
            let copy = path.with_file_name(format!(
                "{name}.ai-stats-backup-{}",
                Utc::now().format("%Y%m%dT%H%M%SZ")
            ));
            fs::copy(&path, &copy)
                .with_context(|| format!("failed to back up {}", path.display()))?;
            backup = Some(copy);
            Some(meta.permissions())
        }
        Err(_) => None,
    };

    let temp = path.with_file_name(format!("{name}.ai-stats-tmp"));
    fs::write(&temp, new_text).with_context(|| format!("failed to write {}", temp.display()))?;
    match mode {
        Some(permissions) => fs::set_permissions(&temp, permissions)?,
        None => crate::paths::restrict_permissions(&temp)?,
    }
    fs::rename(&temp, &path).with_context(|| format!("failed to replace {}", path.display()))?;
    Ok(backup)
}

fn directory_size(files: &[PathBuf]) -> u64 {
    files
        .iter()
        .filter_map(|file| fs::metadata(file).ok())
        .map(|meta| meta.len())
        .sum()
}

fn targets(home: &Path) -> Vec<Target> {
    let env_dir = |name: &str| std::env::var(name).ok().filter(|dir| !dir.is_empty());
    let claude = env_dir("CLAUDE_CONFIG_DIR").map_or_else(|| home.join(".claude"), PathBuf::from);
    let codex = env_dir("CODEX_HOME").map_or_else(|| home.join(".codex"), PathBuf::from);

    let mut found = Vec::new();
    if claude.is_dir() {
        found.push(Target {
            tool: Tool::Claude,
            name: "Claude Code",
            path: claude.join("settings.json"),
        });
    }
    if codex.is_dir() {
        found.push(Target {
            tool: Tool::Codex,
            name: "Codex",
            path: codex.join("config.toml"),
        });
    }
    found
}

fn print_plan(targets: &[Target], edits: &[Edit], log_bytes: u64) {
    println!("This will make your AI CLI logs last longer.\n");
    for (target, edit) in targets.iter().zip(edits) {
        println!("  {}", target.name);
        match edit {
            Edit::Nothing(reason) => println!("    No change needed: {reason}."),
            Edit::Manual(note) => println!("    Not changed: {note}."),
            Edit::Change { summary, .. } => {
                println!("    File:   {}", target.path.display());
                println!("    Change: {summary}");
            }
        }
        if matches!(target.tool, Tool::Codex) {
            println!("    Note:   Codex has no session cleanup setting, so its sessions are kept.");
        }
    }
    println!();
    println!("Each changed file is backed up next to itself first.");
    println!("Only your own settings change. Other users on this host are not affected.");
    println!("Transcripts that were already deleted cannot be brought back.");
    println!(
        "Logs will use more disk over time: your Claude Code transcripts are {}B today.",
        fmt_compact(log_bytes)
    );
    println!();
}

pub fn run(days: u64) -> Result<()> {
    let user = discover::current_user().context("could not determine your home directory")?;
    let targets = targets(&user.home);
    if targets.is_empty() {
        println!("Neither Claude Code nor Codex was found for this user. Nothing to do.");
        return Ok(());
    }

    let edits = targets
        .iter()
        .map(|target| plan(target, days))
        .collect::<Result<Vec<_>>>()?;
    let logs = discover::find_logs(&user, true);
    print_plan(&targets, &edits, directory_size(&logs.claude_sessions));

    if !edits.iter().any(|edit| matches!(edit, Edit::Change { .. })) {
        println!("Everything is already set. Nothing was changed.");
        return Ok(());
    }

    if !io::stdin().is_terminal() {
        bail!("this needs a person at a terminal: run it interactively and type \"yes\"");
    }
    print!("Are you sure? Type \"yes\" and press Enter to continue: ");
    io::stdout().flush()?;
    if !confirmed(&mut io::stdin().lock())? {
        println!("Cancelled. Nothing was changed.");
        return Ok(());
    }

    for target in &targets {
        // Plan again: a running tool may have saved this file during the prompt.
        if let Edit::Change { summary, new_text } = plan(target, days)? {
            let backup = apply(&target.path, &new_text)?;
            println!("{}: {summary}", target.name);
            if let Some(backup) = backup {
                println!("  Backup: {}", backup.display());
            }
        }
    }
    println!("Done. Claude Code picks this up the next time it starts.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ai-stats-retention-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn change(edit: Edit) -> (String, String) {
        match edit {
            Edit::Change { summary, new_text } => (summary, new_text),
            other => panic!("expected a change, got {other:?}"),
        }
    }

    #[test]
    fn adds_the_key_and_keeps_everything_else_in_order() {
        let before = "{\n  \"zebra\": 1,\n  \"hooks\": {\n    \"b\": [],\n    \"a\": \"é\"\n  },\n  \"alpha\": true\n}\n";
        let (summary, after) = change(claude_edit(Some(before), 3650).unwrap());
        assert_eq!(
            summary,
            "cleanupPeriodDays: not set (default 30 days) -> 3650 days"
        );
        assert_eq!(
            after,
            "{\n  \"zebra\": 1,\n  \"hooks\": {\n    \"b\": [],\n    \"a\": \"é\"\n  },\n  \"alpha\": true,\n  \"cleanupPeriodDays\": 3650\n}\n"
        );
    }

    #[test]
    fn creates_settings_when_there_are_none() {
        let (_, after) = change(claude_edit(None, 90).unwrap());
        let value: Value = serde_json::from_str(&after).unwrap();
        assert_eq!(value[CLAUDE_KEY], 90);
    }

    #[test]
    fn raises_a_lower_value_but_never_lowers_a_higher_one() {
        let (summary, _) = change(claude_edit(Some(r#"{"cleanupPeriodDays": 60}"#), 90).unwrap());
        assert!(summary.contains("60 days -> 90 days"));
        assert!(matches!(
            claude_edit(Some(r#"{"cleanupPeriodDays": 9000}"#), 90).unwrap(),
            Edit::Nothing(_)
        ));
    }

    #[test]
    fn broken_settings_are_refused_not_overwritten() {
        assert!(claude_edit(Some("{ not json"), 90).is_err());
        assert!(claude_edit(Some("[1, 2]"), 90).is_err());
    }

    #[test]
    fn codex_limits_are_removed() {
        let before = "model = \"x\"\n\n[history]\npersistence = \"none\"\nmax_bytes = 1000\n\n[tui]\nmax_bytes = 5\n";
        let (summary, after) = change(codex_edit(Some(before)));
        assert_eq!(
            after,
            "model = \"x\"\n\n[history]\npersistence = \"save-all\"\n# max_bytes = 1000\n\n[tui]\nmax_bytes = 5\n"
        );
        assert!(summary.contains("save-all"));
        assert!(summary.contains("limit removed"));
    }

    #[test]
    fn codex_dotted_keys_are_handled() {
        let (_, after) = change(codex_edit(Some("history.persistence = \"none\"\n")));
        assert_eq!(after, "history.persistence = \"save-all\"\n");
    }

    #[test]
    fn codex_with_no_limits_is_left_alone() {
        for text in [
            None,
            Some("model = \"x\"\n"),
            Some("[history]\npersistence = \"save-all\"\n"),
            Some("# persistence = \"none\"\n"),
        ] {
            assert!(matches!(codex_edit(text), Edit::Nothing(_)), "{text:?}");
        }
    }

    #[test]
    fn codex_inline_tables_are_left_for_the_user() {
        let edit = codex_edit(Some("history = { persistence = \"none\" }\n"));
        assert!(matches!(edit, Edit::Manual(_)));
    }

    #[test]
    fn only_a_typed_yes_confirms() {
        assert!(confirmed(&mut Cursor::new("yes\n")).unwrap());
        for answer in ["no\n", "y\n", "YES\n", "yes please\n", "\n", ""] {
            assert!(!confirmed(&mut Cursor::new(answer)).unwrap(), "{answer:?}");
        }
    }

    #[test]
    fn apply_backs_up_and_preserves_the_file_mode() {
        let dir = temp_dir("apply");
        let path = dir.join("settings.json");
        fs::write(&path, "old").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        }

        let backup = apply(&path, "new").unwrap().unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(fs::read_to_string(backup).unwrap(), "old");
        assert!(!dir.join("settings.json.ai-stats-tmp").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o640);
        }
    }

    #[cfg(unix)]
    #[test]
    fn apply_writes_through_a_symlink() {
        let dir = temp_dir("symlink");
        let real = dir.join("real.json");
        let link = dir.join("settings.json");
        fs::write(&real, "old").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();

        apply(&link, "new").unwrap();

        assert_eq!(fs::read_to_string(&real).unwrap(), "new");
        assert!(fs::symlink_metadata(&link).unwrap().is_symlink());
    }

    #[test]
    fn missing_tools_are_not_created() {
        let home = temp_dir("empty-home");
        assert!(targets(&home).is_empty());
        assert!(!home.join(".claude").exists());
    }
}
