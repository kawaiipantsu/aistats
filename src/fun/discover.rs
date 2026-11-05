//! Works out whose chat logs to read and reads them defensively.
//!
//! Run as root this walks into directories other users control, so nothing
//! found there is trusted: symlinks are never followed, only regular files
//! are opened (a planted FIFO must not hang the scan), recursion is bounded,
//! and a single line cannot make the process buffer an unbounded amount.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};

/// Longest JSONL record that is parsed. Real records top out around 7 MiB
/// (inline images); anything larger is counted as skipped rather than loaded.
pub const MAX_LINE_BYTES: usize = 16 * 1024 * 1024;

/// Deepest directory level searched below a log root. Codex nests by
/// `YYYY/MM/DD`, Claude by `<project>/<session>/subagents`.
const MAX_DEPTH: usize = 6;

/// A local account to scan. `name` is only ever used to derive a pseudonym; it
/// must never be written to report output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostUser {
    pub name: String,
    pub uid: Option<u32>,
    pub home: PathBuf,
}

/// All the log locations belonging to one user.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogSet {
    pub claude_sessions: Vec<PathBuf>,
    pub codex_sessions: Vec<PathBuf>,
    pub claude_history: Option<PathBuf>,
    pub codex_history: Option<PathBuf>,
}

impl LogSet {
    pub fn is_empty(&self) -> bool {
        self.claude_sessions.is_empty()
            && self.codex_sessions.is_empty()
            && self.claude_history.is_none()
            && self.codex_history.is_none()
    }
}

#[cfg(unix)]
fn current_uid() -> Option<u32> {
    // SAFETY: geteuid takes no arguments, cannot fail, and touches no memory.
    Some(unsafe { libc::geteuid() })
}

#[cfg(not(unix))]
fn current_uid() -> Option<u32> {
    None
}

pub fn is_root() -> bool {
    current_uid() == Some(0)
}

/// Parses `/etc/passwd`, skipping accounts with no usable home directory.
/// System accounts are kept on purpose: service users such as `service-example` often
/// have the busiest logs and homes outside `/home`.
pub fn parse_passwd(text: &str) -> Vec<HostUser> {
    text.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split(':').collect();
            let (name, uid, home) = (*fields.first()?, fields.get(2)?, *fields.get(5)?);
            if name.is_empty() || !home.starts_with('/') || home == "/" {
                return None;
            }
            Some(HostUser {
                name: name.to_string(),
                uid: uid.parse().ok(),
                home: PathBuf::from(home),
            })
        })
        .collect()
}

/// The accounts this process is allowed to read.
pub fn users_to_scan() -> Vec<HostUser> {
    let accounts = fs::read_to_string("/etc/passwd")
        .map(|text| parse_passwd(&text))
        .unwrap_or_default();
    select_users(
        accounts,
        current_uid(),
        dirs::home_dir(),
        Path::new("/home"),
    )
}

/// The invoking account. Looked up by uid first, since `$HOME` can still point
/// at the original user under `sudo`.
pub fn current_user() -> Option<HostUser> {
    let accounts = fs::read_to_string("/etc/passwd")
        .map(|text| parse_passwd(&text))
        .unwrap_or_default();
    find_me(accounts, current_uid(), dirs::home_dir())
}

fn find_me(
    accounts: Vec<HostUser>,
    uid: Option<u32>,
    own_home: Option<PathBuf>,
) -> Option<HostUser> {
    accounts
        .into_iter()
        .find(|user| uid.is_some() && user.uid == uid)
        .or_else(|| {
            own_home.map(|home| HostUser {
                name: std::env::var("USER").unwrap_or_else(|_| "me".to_string()),
                uid,
                home,
            })
        })
}

/// Everyone when running as root, otherwise only the caller, however
/// permissive other users' home directories happen to be.
fn select_users(
    accounts: Vec<HostUser>,
    uid: Option<u32>,
    own_home: Option<PathBuf>,
    home_root: &Path,
) -> Vec<HostUser> {
    if uid == Some(0) {
        return all_users(accounts, home_root);
    }
    find_me(accounts, uid, own_home).into_iter().collect()
}

/// Adds directories under `home_root` that have no passwd entry (LDAP or
/// otherwise directory-managed accounts), then drops accounts sharing a home.
fn all_users(mut users: Vec<HostUser>, home_root: &Path) -> Vec<HostUser> {
    if let Ok(entries) = fs::read_dir(home_root) {
        for entry in entries.flatten() {
            let is_dir = entry.file_type().is_ok_and(|kind| kind.is_dir());
            if is_dir && !users.iter().any(|user| user.home == entry.path()) {
                users.push(HostUser {
                    name: entry.file_name().to_string_lossy().into_owned(),
                    uid: None,
                    home: entry.path(),
                });
            }
        }
    }
    let mut seen = HashSet::new();
    users.retain(|user| seen.insert(user.home.clone()));
    users
}

/// Locates a user's logs. `trusted` allows the log root itself to be a
/// symlink, which is fine for the invoking user's own directories but not for
/// anyone else's: as root that would let a user point the scan at files they
/// cannot read.
pub fn find_logs(user: &HostUser, trusted: bool) -> LogSet {
    let claude_roots = claude_roots(&user.home, trusted);
    let codex_roots = codex_roots(&user.home, trusted);

    let mut logs = LogSet::default();
    for root in &claude_roots {
        collect_jsonl(&root.join("projects"), trusted, &mut logs.claude_sessions);
        logs.claude_history = logs
            .claude_history
            .or_else(|| regular_file(&root.join("history.jsonl")));
    }
    for root in &codex_roots {
        collect_jsonl(&root.join("sessions"), trusted, &mut logs.codex_sessions);
        collect_jsonl(
            &root.join("archived_sessions"),
            trusted,
            &mut logs.codex_sessions,
        );
        logs.codex_history = logs
            .codex_history
            .or_else(|| regular_file(&root.join("history.jsonl")));
    }
    logs
}

fn claude_roots(home: &Path, trusted: bool) -> Vec<PathBuf> {
    let mut roots = vec![home.join(".claude"), home.join(".config").join("claude")];
    if trusted && let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        roots.push(PathBuf::from(dir));
    }
    roots
}

fn codex_roots(home: &Path, trusted: bool) -> Vec<PathBuf> {
    let mut roots = vec![home.join(".codex")];
    if trusted && let Ok(dir) = std::env::var("CODEX_HOME") {
        roots.push(PathBuf::from(dir));
    }
    roots
}

fn regular_file(path: &Path) -> Option<PathBuf> {
    fs::symlink_metadata(path)
        .is_ok_and(|meta| meta.is_file())
        .then(|| path.to_path_buf())
}

fn collect_jsonl(root: &Path, trusted: bool, out: &mut Vec<PathBuf>) {
    let meta = if trusted {
        fs::metadata(root)
    } else {
        fs::symlink_metadata(root)
    };
    if meta.is_ok_and(|meta| meta.is_dir()) {
        walk(root, 0, out);
    }
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        // DirEntry::file_type does not follow symlinks, so links are neither
        // descended into nor collected.
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            walk(&path, depth + 1, out);
        } else if kind.is_file() && path.extension().is_some_and(|ext| ext == "jsonl") {
            out.push(path);
        }
    }
}

/// Opens a log without following a final symlink and without blocking on a
/// FIFO, then confirms what was actually opened. Checking the open descriptor
/// rather than the path closes the race where a file is swapped between
/// discovery and this call.
pub fn open_log(path: &Path) -> io::Result<File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("not a regular file"));
    }
    Ok(file)
}

/// Calls `on_line` for each newline-terminated record, returning how many were
/// skipped for exceeding `max_line`. Oversized records are discarded as they
/// stream past, so memory stays bounded by `max_line`.
pub fn for_each_line(
    file: File,
    max_line: usize,
    mut on_line: impl FnMut(&[u8]),
) -> io::Result<u64> {
    let mut reader = BufReader::with_capacity(1024 * 1024, file);
    let mut line = Vec::new();
    let mut oversized = false;
    let mut skipped = 0;

    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            if oversized {
                skipped += 1;
            } else if !line.is_empty() {
                on_line(&line);
            }
            return Ok(skipped);
        }

        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let chunk = &buffer[..newline.unwrap_or(buffer.len())];
        if !oversized {
            if line.len() + chunk.len() > max_line {
                oversized = true;
                line = Vec::new();
            } else {
                line.extend_from_slice(chunk);
            }
        }

        let used = chunk.len() + usize::from(newline.is_some());
        reader.consume(used);

        if newline.is_some() {
            if oversized {
                skipped += 1;
            } else {
                on_line(&line);
            }
            oversized = false;
            line.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ai-stats-discover-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn passwd_keeps_service_accounts_outside_home() {
        let users = parse_passwd(
            "root:x:0:0:root:/root:/usr/bin/zsh\n\
             service-example:x:995:995::/var/lib/service-example:/bin/bash\n\
             rootless:x:1:1::/:/bin/false\n\
             garbled-line\n",
        );
        let homes: Vec<_> = users.iter().map(|u| u.home.to_str().unwrap()).collect();
        assert_eq!(homes, ["/root", "/var/lib/service-example"]);
        assert_eq!(users[1].uid, Some(995));
    }

    fn accounts() -> Vec<HostUser> {
        parse_passwd(
            "root:x:0:0::/root:/bin/sh\n\
             alice:x:1000:1000::/home/alice:/bin/sh\n\
             service-example:x:995:995::/var/lib/service-example:/bin/bash\n",
        )
    }

    #[test]
    fn root_scans_every_account() {
        let users = select_users(accounts(), Some(0), None, Path::new("/nonexistent"));
        assert_eq!(users.len(), 3);
    }

    #[test]
    fn root_also_finds_homes_missing_from_passwd() {
        let home_root = temp_dir("home-root");
        fs::create_dir(home_root.join("ldap-user")).unwrap();
        let users = select_users(accounts(), Some(0), None, &home_root);
        assert!(users.iter().any(|user| user.name == "ldap-user"));
    }

    #[test]
    fn non_root_scans_only_itself() {
        let users = select_users(accounts(), Some(1000), None, Path::new("/home"));
        let names: Vec<_> = users.iter().map(|user| user.name.as_str()).collect();
        assert_eq!(names, ["alice"]);
    }

    #[test]
    fn non_root_without_a_passwd_entry_falls_back_to_its_own_home() {
        let home = PathBuf::from("/tmp/somewhere");
        let users = select_users(
            accounts(),
            Some(4242),
            Some(home.clone()),
            Path::new("/home"),
        );
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].home, home);
    }

    #[test]
    fn users_sharing_a_home_are_scanned_once() {
        let users = vec![
            HostUser {
                name: "root".into(),
                uid: Some(0),
                home: "/root".into(),
            },
            HostUser {
                name: "toor".into(),
                uid: Some(0),
                home: "/root".into(),
            },
        ];
        assert_eq!(all_users(users, Path::new("/nonexistent")).len(), 1);
    }

    #[test]
    fn discovers_sessions_from_both_tools() {
        let home = temp_dir("both");
        let claude = home.join(".claude/projects/some-project");
        let codex = home.join(".codex/sessions/2026/10/31");
        fs::create_dir_all(&claude).unwrap();
        fs::create_dir_all(&codex).unwrap();
        fs::write(claude.join("a.jsonl"), "{}\n").unwrap();
        fs::write(claude.join("notes.md"), "ignored").unwrap();
        fs::write(codex.join("rollout.jsonl"), "{}\n").unwrap();
        fs::write(home.join(".codex/history.jsonl"), "{}\n").unwrap();

        let user = HostUser {
            name: "someone".into(),
            uid: None,
            home: home.clone(),
        };
        let logs = find_logs(&user, false);
        assert_eq!(logs.claude_sessions.len(), 1);
        assert_eq!(logs.codex_sessions.len(), 1);
        assert_eq!(logs.codex_history, Some(home.join(".codex/history.jsonl")));
        assert_eq!(logs.claude_history, None);
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_planted_by_other_users_are_not_followed() {
        use std::os::unix::fs::symlink;

        let home = temp_dir("symlink");
        let elsewhere = temp_dir("symlink-target");
        fs::write(elsewhere.join("victim.jsonl"), "{}\n").unwrap();

        let projects = home.join(".claude/projects");
        fs::create_dir_all(&projects).unwrap();
        symlink(elsewhere.join("victim.jsonl"), projects.join("file.jsonl")).unwrap();
        symlink(&elsewhere, projects.join("dir")).unwrap();
        symlink(&elsewhere, home.join(".codex")).unwrap();

        let user = HostUser {
            name: "mallory".into(),
            uid: None,
            home,
        };
        assert!(find_logs(&user, false).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn open_log_refuses_symlinks_and_fifos() {
        use std::os::unix::fs::symlink;

        let dir = temp_dir("open");
        let real = dir.join("real.jsonl");
        fs::write(&real, "{}\n").unwrap();
        let link = dir.join("link.jsonl");
        symlink(&real, &link).unwrap();
        let fifo = dir.join("fifo.jsonl");
        let path = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        // SAFETY: `path` is a valid NUL-terminated string for the whole call.
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);

        assert!(open_log(&real).is_ok());
        assert!(open_log(&link).is_err());
        // Would block forever without O_NONBLOCK.
        assert!(open_log(&fifo).is_err());
    }

    #[test]
    fn long_lines_are_skipped_without_losing_neighbours() {
        let dir = temp_dir("lines");
        let path = dir.join("big.jsonl");
        let mut file = File::create(&path).unwrap();
        writeln!(file, "short one").unwrap();
        writeln!(file, "{}", "x".repeat(100)).unwrap();
        write!(file, "no trailing newline").unwrap();
        drop(file);

        let mut seen = Vec::new();
        let skipped = for_each_line(File::open(&path).unwrap(), 50, |line| {
            seen.push(String::from_utf8_lossy(line).into_owned());
        })
        .unwrap();

        assert_eq!(seen, ["short one", "no trailing newline"]);
        assert_eq!(skipped, 1);
    }
}
