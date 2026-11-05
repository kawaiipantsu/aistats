//! Optional OpenSSH collection. Raw logs live only in private temporary
//! directories; remote paths never determine where a file is written locally.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use tempfile::TempDir;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use super::discover::{HostUser, LogSet};

const EXPORTER: &str = include_str!("remote.py");
const MAX_HEADER: u64 = 16 * 1024;
const MAX_FILES: usize = 100_000;
const MAX_USERS: usize = 10_000;
const MAX_BYTES: u64 = 32 * 1024 * 1024 * 1024;

pub struct Snapshot {
    pub host: String,
    pub users: Vec<(HostUser, LogSet)>,
    pub all_users: bool,
    pub unreadable: u64,
    // Dropped last, after all paths are no longer in use.
    _directory: TempDir,
}

/// Accept destinations, never arbitrary SSH options, shell syntax or URLs.
/// Lowercase hosts so repeated spelling/case variants are scanned only once.
pub fn parse_host(value: &str) -> std::result::Result<String, String> {
    let invalid =
        || "expected user@hostname or user@IP (no spaces, options or port suffix)".to_owned();
    let (user, host) = value.split_once('@').ok_or_else(invalid)?;
    if user.is_empty()
        || !user
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
        || user.starts_with('-')
        || host.is_empty()
        || host.len() > 253
    {
        return Err(invalid());
    }
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    let valid = if host.contains(':') {
        host.parse::<std::net::Ipv6Addr>().is_ok()
    } else {
        host.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            && host
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphanumeric)
    };
    if !valid {
        return Err(invalid());
    }
    Ok(format!("{user}@{}", host.to_ascii_lowercase()))
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Header {
    Hello {
        version: u32,
        all_users: bool,
    },
    User {
        name: String,
        home: PathBuf,
    },
    File {
        kind: LogKind,
        name: String,
        size: u64,
    },
    Done {
        unreadable: u64,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum LogKind {
    ClaudeSession,
    CodexSession,
    ClaudeHistory,
    CodexHistory,
}

async fn header(reader: &mut (impl AsyncBufRead + Unpin)) -> Result<Header> {
    let mut bytes = Vec::new();
    (&mut *reader)
        .take(MAX_HEADER + 1)
        .read_until(b'\n', &mut bytes)
        .await?;
    ensure!(
        bytes.len() as u64 <= MAX_HEADER && bytes.last() == Some(&b'\n'),
        "invalid or incomplete SSH export header"
    );
    // serde's error may quote remote text: only return our fixed message.
    serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("invalid SSH export metadata"))
}

async fn receive(reader: &mut (impl AsyncBufRead + Unpin)) -> Result<Snapshot> {
    let Header::Hello {
        version: 1,
        all_users,
    } = header(reader).await?
    else {
        bail!("unsupported SSH export protocol (remote host needs Python 3)");
    };
    let mut directory = tempfile::Builder::new();
    directory.prefix("ai-stats-ssh-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        directory.permissions(std::fs::Permissions::from_mode(0o700));
    }
    let directory = directory.tempdir()?;
    let mut users: Vec<(HostUser, LogSet)> = Vec::new();
    let mut names = HashSet::new();
    let mut files = 0;
    let mut bytes = 0_u64;
    let unreadable;
    loop {
        match header(reader).await? {
            Header::User { name, home } => {
                ensure!(
                    users.len() < MAX_USERS
                        && !name.is_empty()
                        && names.insert(name.clone())
                        && home.is_absolute(),
                    "invalid or excessive SSH export users"
                );
                users.push((
                    HostUser {
                        name,
                        home,
                        uid: None,
                    },
                    LogSet::default(),
                ));
            }
            Header::File { kind, name, size } => {
                // Preserve only the basename for Claude's history/session
                // deduplication. Every file gets a locally numbered directory.
                ensure!(
                    !name.is_empty()
                        && name.len() <= 255
                        && name != "."
                        && name != ".."
                        && !name.contains(['/', '\\', '\0'])
                        && name.ends_with(".jsonl"),
                    "invalid SSH log filename"
                );
                bytes = bytes.checked_add(size).context("SSH export too large")?;
                ensure!(
                    files < MAX_FILES && bytes <= MAX_BYTES,
                    "SSH export exceeds 100,000 files or 32 GiB per host"
                );
                let (_, logs) = users
                    .last_mut()
                    .context("SSH file arrived before its user")?;
                let parent = directory.path().join(files.to_string());
                tokio::fs::create_dir(&parent).await?;
                let path = parent.join(name);
                let mut file = tokio::fs::File::create(&path).await?;
                let copied = tokio::io::copy(&mut (&mut *reader).take(size), &mut file).await?;
                ensure!(copied == size, "SSH log transfer ended early");
                file.flush().await?;
                match kind {
                    LogKind::ClaudeSession => logs.claude_sessions.push(path),
                    LogKind::CodexSession => logs.codex_sessions.push(path),
                    LogKind::ClaudeHistory => ensure!(
                        logs.claude_history.replace(path).is_none(),
                        "duplicate Claude history in SSH export"
                    ),
                    LogKind::CodexHistory => ensure!(
                        logs.codex_history.replace(path).is_none(),
                        "duplicate Codex history in SSH export"
                    ),
                }
                files += 1;
            }
            Header::Done { unreadable: count } => {
                unreadable = count;
                break;
            }
            Header::Hello { .. } => bail!("unexpected SSH export header"),
        }
    }
    ensure!(
        reader.fill_buf().await?.is_empty(),
        "unexpected data after SSH export"
    );
    Ok(Snapshot {
        host: String::new(),
        users,
        all_users,
        unreadable,
        _directory: directory,
    })
}

async fn collect_host(host: &str, key: &Path) -> Result<Snapshot> {
    let mut child = Command::new("ssh")
        .args([
            "-T",
            "-o",
            "BatchMode=yes",
            "-o",
            "IdentitiesOnly=yes",
            "-o",
            "StrictHostKeyChecking=yes",
            "-o",
            "ConnectTimeout=15",
            "-o",
            "ServerAliveInterval=15",
            "-o",
            "ServerAliveCountMax=2",
            "-o",
            "ForwardAgent=no",
            "-o",
            "ClearAllForwardings=yes",
            "-i",
        ])
        .arg(key)
        .arg(host)
        // Constant command; no host, paths or other user input is interpolated
        // into a remote shell. Python reads the bundled script from stdin.
        .arg("python3 -")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .context("could not start OpenSSH; install the ssh client")?;
    let mut input = child.stdin.take().context("SSH stdin unavailable")?;
    input
        .write_all(EXPORTER.as_bytes())
        .await
        .context("could not send SSH exporter")?;
    drop(input);
    let mut output = BufReader::new(child.stdout.take().context("SSH stdout unavailable")?);
    let mut snapshot = receive(&mut output).await.context(
        "could not collect remote logs; check SSH key, known_hosts, access and remote Python 3",
    )?;
    ensure!(
        child.wait().await?.success(),
        "SSH exporter exited unsuccessfully"
    );
    snapshot.host = host.to_string();
    Ok(snapshot)
}

pub async fn collect(
    hosts: &[String],
    key: Option<&Path>,
    timeout: Duration,
) -> Result<Vec<Snapshot>> {
    if hosts.is_empty() {
        return Ok(Vec::new());
    }
    let key = key.context("--ssh-host requires --ssh-key")?;
    // Absolute paths prevent a filename beginning with '-' being an option.
    let key = std::fs::canonicalize(key).context("cannot open the SSH private key path")?;
    ensure!(key.is_file(), "SSH private key must be a file");
    let mut seen = HashSet::new();
    let mut snapshots = Vec::new();
    for host in hosts {
        let host = parse_host(host).map_err(anyhow::Error::msg)?;
        if !seen.insert(host.clone()) {
            continue;
        }
        let number = snapshots.len() + 1;
        eprintln!("Collecting SSH host {number}...");
        let snapshot = tokio::time::timeout(timeout, collect_host(&host, &key))
            .await
            .with_context(|| {
                format!(
                    "SSH host {number} exceeded --ssh-timeout ({} seconds)",
                    timeout.as_secs()
                )
            })?
            .with_context(|| {
                format!("SSH host {number} collection failed; no report was exported")
            })?;
        snapshots.push(snapshot);
    }
    Ok(snapshots)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn frame(out: &mut Vec<u8>, value: serde_json::Value) {
        serde_json::to_writer(&mut *out, &value).unwrap();
        out.push(b'\n');
    }

    fn start() -> Vec<u8> {
        let mut out = Vec::new();
        frame(
            &mut out,
            json!({"type":"hello", "version":1, "all_users":true}),
        );
        frame(
            &mut out,
            json!({"type":"user", "name":"alice", "home":"/home/alice"}),
        );
        out
    }

    #[test]
    fn ssh_destinations_cannot_supply_options_or_shell_syntax() {
        for value in [
            "-oProxyCommand=bad",
            "root@host;id",
            "root@host\n",
            "root@$(id)",
            "root@host:2222",
            "root@",
            "@host",
            "root@@host",
            "root@-option",
            "-root@host",
            "root@host name",
            "ssh://root@host",
            "root@host%h",
        ] {
            assert!(parse_host(value).is_err(), "accepted {value:?}");
        }
        assert_eq!(
            parse_host("alice@EXAMPLE.com").unwrap(),
            "alice@example.com"
        );
        assert_eq!(
            parse_host("alice@[2001:db8::1]").unwrap(),
            "alice@2001:db8::1"
        );
        assert!(parse_host("alice@192.0.2.1").is_ok());
    }

    #[tokio::test]
    async fn local_only_needs_neither_key_nor_ssh() {
        assert!(
            collect(&[], None, Duration::from_secs(1))
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn receives_private_snapshots_then_removes_raw_logs() {
        let mut stream = start();
        let content = b"{\"private\":\"not report text\"}\n";
        frame(
            &mut stream,
            json!({"type":"file", "kind":"claude_session", "name":"session.jsonl", "size":content.len()}),
        );
        stream.extend_from_slice(content);
        frame(&mut stream, json!({"type":"done", "unreadable":2}));
        let snapshot = receive(&mut stream.as_slice()).await.unwrap();
        let directory = snapshot._directory.path().to_path_buf();
        let path = &snapshot.users[0].1.claude_sessions[0];
        assert_eq!(std::fs::read(path).unwrap(), content);
        assert_eq!(path.file_stem().unwrap(), "session");
        assert_eq!(snapshot.unreadable, 2);
        assert!(snapshot.all_users);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        drop(snapshot);
        assert!(!directory.exists());
    }

    #[tokio::test]
    async fn refuses_traversal_truncation_overflows_and_extra_output() {
        for name in [
            "../escape.jsonl",
            "/tmp/escape.jsonl",
            "a\\escape.jsonl",
            "bad\0.jsonl",
            "config.toml",
        ] {
            let mut stream = start();
            frame(
                &mut stream,
                json!({"type":"file", "kind":"codex_session", "name":name, "size":0}),
            );
            assert!(receive(&mut stream.as_slice()).await.is_err());
        }
        for size in [20, u64::MAX] {
            let mut stream = start();
            frame(
                &mut stream,
                json!({"type":"file", "kind":"codex_session", "name":"ok.jsonl", "size":size}),
            );
            stream.extend_from_slice(b"short");
            assert!(receive(&mut stream.as_slice()).await.is_err());
        }
        let mut stream = start();
        frame(&mut stream, json!({"type":"done", "unreadable":0}));
        stream.extend_from_slice(b"unexpected");
        assert!(receive(&mut stream.as_slice()).await.is_err());
        let oversized = vec![b'x'; MAX_HEADER as usize + 2];
        assert!(receive(&mut oversized.as_slice()).await.is_err());
    }

    #[tokio::test]
    async fn remote_metadata_errors_never_echo_private_text() {
        let error = receive(&mut b"{\"type\":\"private-secret-value\"}\n".as_slice())
            .await
            .err()
            .unwrap();
        assert!(!format!("{error:#}").contains("private-secret-value"));
    }
}
