//! `--fun-stats`: a PISG-style statistics report built from local Claude Code
//! and Codex chat logs.
//!
//! Nothing here touches the usage cache or any provider API. The data flow is
//! `discover` (which logs) -> `parse` (records to events) -> `analyze` (events
//! to counters) -> [`FunReport`], which `crate::report::fun` renders.

pub mod analyze;
pub mod discover;
pub mod lexicon;
pub mod names;
pub mod parse;
pub mod remote;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{BuildHasher, DefaultHasher, Hash, Hasher, RandomState};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;

use analyze::{QuoteCandidate, SessionAnalyzer, Tally};
use discover::{HostUser, LogSet};
use lexicon::{Cat, Lexicon};
use names::{NameBook, NameRoasts};
use parse::{ClaudeParser, CodexParser, Event};

const MAX_QUOTES: usize = 5;
const MAX_ROASTS: usize = 3;

const ADJECTIVES: &[&str] = &[
    "Sleepy",
    "Grumpy",
    "Caffeinated",
    "Sneaky",
    "Wobbly",
    "Fearless",
    "Dramatic",
    "Sarcastic",
    "Chaotic",
    "Polite",
    "Feral",
    "Tiny",
    "Mighty",
    "Confused",
    "Stoic",
    "Spicy",
    "Cosmic",
    "Rusty",
    "Turbo",
    "Humble",
    "Salty",
    "Fluffy",
    "Nocturnal",
    "Unhinged",
    "Zen",
    "Bewildered",
    "Crispy",
    "Majestic",
    "Suspicious",
    "Jolly",
    "Shy",
    "Reckless",
];

const ANIMALS: &[(&str, &str)] = &[
    ("🦦", "Otter"),
    ("🦊", "Fox"),
    ("🦉", "Owl"),
    ("🐙", "Octopus"),
    ("🦥", "Sloth"),
    ("🦝", "Raccoon"),
    ("🐧", "Penguin"),
    ("🦆", "Duck"),
    ("🐢", "Turtle"),
    ("🐝", "Bee"),
    ("🦄", "Unicorn"),
    ("🦖", "T-Rex"),
    ("🐳", "Whale"),
    ("🦔", "Hedgehog"),
    ("🐐", "Goat"),
    ("🦙", "Llama"),
];

/// Which of the host's users a run covered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    AllUsers,
    CurrentUser,
    MixedUsers,
}

/// How much of the logs were readable, so a thin report can be explained.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Coverage {
    pub hosts_scanned: u64,
    pub hosts_all_users: u64,
    pub files_read: u64,
    pub files_unreadable: u64,
    pub records_malformed: u64,
    pub records_oversized: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct UserStats {
    /// Generated pseudonym. Real account names never reach the report.
    pub name: String,
    pub tally: Tally,
}

#[derive(Debug, Clone, Serialize)]
pub struct FunReport {
    pub generated_at: DateTime<Utc>,
    pub scope: Scope,
    pub coverage: Coverage,
    pub total: Tally,
    pub by_tool: BTreeMap<&'static str, Tally>,
    pub by_user: Vec<UserStats>,
    pub quotes: Vec<String>,
    pub names: NameRoasts,
}

#[derive(Debug, Clone, Copy)]
pub struct Range {
    pub since: DateTime<Utc>,
    pub until: DateTime<Utc>,
}

impl Range {
    fn contains(&self, at: Option<DateTime<Utc>>) -> bool {
        at.is_none_or(|at| at >= self.since && at < self.until)
    }
}

/// Assigns each user a pseudonym derived from a salted hash, so names stay
/// stable between runs but cannot be recovered by hashing a list of likely
/// usernames. `names` must be distinct.
pub fn pseudonyms(names: &[String], salt: u128) -> HashMap<String, String> {
    let name_count = ADJECTIVES.len() * ANIMALS.len();
    let mut taken = HashSet::new();
    let mut hashed: Vec<(u64, &String)> = names
        .iter()
        .map(|name| {
            let mut hasher = DefaultHasher::new();
            salt.hash(&mut hasher);
            name.hash(&mut hasher);
            (hasher.finish(), name)
        })
        .collect();
    hashed.sort();

    hashed
        .into_iter()
        .map(|(hash, name)| {
            let mut slot = hash as usize % name_count;
            while !taken.insert(slot) {
                slot = (slot + 1) % name_count;
            }
            let (emoji, animal) = ANIMALS[slot % ANIMALS.len()];
            let adjective = ADJECTIVES[slot / ANIMALS.len()];
            (name.clone(), format!("{emoji} {adjective} {animal}"))
        })
        .collect()
}

pub fn random_salt() -> u128 {
    let state = RandomState::new();
    (u128::from(state.hash_one(1u8)) << 64) | u128::from(state.hash_one(2u8))
}

/// A prompt recovered from a history file, with when it was sent.
type HistoryPrompt = (String, Option<DateTime<Utc>>);

/// Everything collected for one user.
#[derive(Default)]
struct UserScan {
    by_tool: BTreeMap<&'static str, Tally>,
    quotes: Vec<QuoteCandidate>,
    names: NameBook,
}

struct Scanner<'a> {
    lexicon: &'a Lexicon,
    range: Range,
    coverage: Coverage,
    /// Hashes of record ids, so a session copied into a second file by a
    /// resume or fork is not counted twice.
    seen_records: HashSet<u64>,
}

impl Scanner<'_> {
    fn read_records(&mut self, path: &Path, mut on_record: impl FnMut(&mut Coverage, &[u8])) {
        let file = match discover::open_log(path) {
            Ok(file) => file,
            Err(_) => {
                self.coverage.files_unreadable += 1;
                return;
            }
        };
        let mut coverage = std::mem::take(&mut self.coverage);
        let result = discover::for_each_line(file, discover::MAX_LINE_BYTES, |line| {
            on_record(&mut coverage, line);
        });
        match result {
            Ok(skipped) => {
                coverage.files_read += 1;
                coverage.records_oversized += skipped;
            }
            Err(_) => coverage.files_unreadable += 1,
        }
        self.coverage = coverage;
    }

    fn is_new_record(seen: &mut HashSet<u64>, record: &Value) -> bool {
        parse::record_id(record).is_none_or(|id| {
            let mut hasher = DefaultHasher::new();
            id.hash(&mut hasher);
            seen.insert(hasher.finish())
        })
    }

    fn claude_session(&mut self, path: &Path, scan: &mut UserScan) {
        let lexicon = self.lexicon;
        let range = self.range;
        let mut seen = std::mem::take(&mut self.seen_records);
        let tally = scan.by_tool.entry("claude").or_default();
        let mut session =
            SessionAnalyzer::new(lexicon, "claude", tally, &mut scan.quotes, &mut scan.names);
        let mut parser = ClaudeParser::default();

        self.read_records(path, |coverage, line| {
            let Ok(record) = serde_json::from_slice::<Value>(line) else {
                coverage.records_malformed += 1;
                return;
            };
            let at = parse::timestamp(&record);
            if !range.contains(at) || !Self::is_new_record(&mut seen, &record) {
                return;
            }
            let mut events = Vec::new();
            parser.line(&record, &mut events);
            for event in &events {
                session.event(at, event);
            }
        });

        let mut usage: Vec<Event<'_>> = Vec::new();
        parser.finish(&mut usage);
        for event in &usage {
            session.event(None, event);
        }
        session.finish();
        self.seen_records = seen;
    }

    fn codex_session(&mut self, path: &Path, scan: &mut UserScan) -> Option<String> {
        let lexicon = self.lexicon;
        let range = self.range;
        let mut seen = std::mem::take(&mut self.seen_records);
        let tally = scan.by_tool.entry("codex").or_default();
        let mut session =
            SessionAnalyzer::new(lexicon, "codex", tally, &mut scan.quotes, &mut scan.names);
        let mut parser = CodexParser::default();

        self.read_records(path, |coverage, line| {
            let Ok(record) = serde_json::from_slice::<Value>(line) else {
                coverage.records_malformed += 1;
                return;
            };
            let at = parse::timestamp(&record);
            // The parser sees every record so running totals stay correct;
            // only the resulting events are filtered.
            let mut events = Vec::new();
            parser.line(&record, &mut events);
            if range.contains(at) && Self::is_new_record(&mut seen, &record) {
                for event in &events {
                    session.event(at, event);
                }
            }
        });

        session.finish();
        self.seen_records = seen;
        parser.session_id
    }

    /// Adds prompts from history files for sessions whose transcripts are
    /// gone, grouped by session so per-session stats stay meaningful.
    fn history(
        &mut self,
        path: &Path,
        tool: &'static str,
        known: &HashSet<String>,
        scan: &mut UserScan,
    ) {
        let range = self.range;
        let mut sessions: HashMap<String, Vec<HistoryPrompt>> = HashMap::new();
        self.read_records(path, |coverage, line| {
            let Ok(record) = serde_json::from_slice::<Value>(line) else {
                coverage.records_malformed += 1;
                return;
            };
            if let Some((session, text, at)) = parse::history_entry(&record)
                && !known.contains(session)
                && range.contains(at)
            {
                sessions
                    .entry(session.to_string())
                    .or_default()
                    .push((text.to_string(), at));
            }
        });

        let tally = scan.by_tool.entry(tool).or_default();
        for prompts in sessions.into_values() {
            let mut session =
                SessionAnalyzer::new(self.lexicon, tool, tally, &mut scan.quotes, &mut scan.names);
            for (text, at) in &prompts {
                session.event(*at, &Event::Prompt(text.as_str().into()));
            }
            session.finish();
        }
    }

    fn user(&mut self, user: &HostUser, logs: &LogSet) -> UserScan {
        let mut scan = UserScan {
            names: NameBook::for_user(&user.home, &user.name),
            ..UserScan::default()
        };

        let mut known_claude = HashSet::new();
        for path in &logs.claude_sessions {
            self.claude_session(path, &mut scan);
            if let Some(stem) = path.file_stem() {
                known_claude.insert(stem.to_string_lossy().into_owned());
            }
        }
        let mut known_codex = HashSet::new();
        for path in &logs.codex_sessions {
            known_codex.extend(self.codex_session(path, &mut scan));
        }

        if let Some(path) = &logs.claude_history {
            self.history(path, "claude", &known_claude, &mut scan);
        }
        if let Some(path) = &logs.codex_history {
            self.history(path, "codex", &known_codex, &mut scan);
        }
        scan
    }
}

fn pick_quotes(mut candidates: Vec<QuoteCandidate>) -> Vec<String> {
    candidates.sort_by(|a, b| {
        b.spice
            .cmp(&a.spice)
            .then(a.words.len().cmp(&b.words.len()))
            .then(a.words.cmp(&b.words))
    });
    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .map(|quote| quote.words.join(" "))
        .filter(|quote| seen.insert(quote.clone()))
        .take(MAX_QUOTES)
        .collect()
}

/// Scans the given users. Takes the log locations explicitly so everything
/// except user discovery can be exercised without touching real homes.
#[cfg(test)]
pub fn build_report(
    users: &[(HostUser, LogSet)],
    scope: Scope,
    range: Range,
    salt: u128,
) -> FunReport {
    let names: Vec<String> = users.iter().map(|(user, _)| user.name.clone()).collect();
    build_report_with_ids(users, &names, scope, range, salt)
}

fn build_report_with_ids(
    users: &[(HostUser, LogSet)],
    identities: &[String],
    scope: Scope,
    range: Range,
    salt: u128,
) -> FunReport {
    let lexicon = Lexicon::new();
    let pseudonyms = pseudonyms(identities, salt);

    let mut scanner = Scanner {
        lexicon: &lexicon,
        range,
        coverage: Coverage {
            hosts_scanned: 1,
            hosts_all_users: u64::from(scope == Scope::AllUsers),
            ..Coverage::default()
        },
        seen_records: HashSet::new(),
    };

    let mut total = Tally::default();
    let mut by_tool: BTreeMap<&'static str, Tally> = BTreeMap::new();
    let mut by_user = Vec::new();
    let mut quotes = Vec::new();
    let mut names = NameBook::default();

    for ((user, logs), identity) in users.iter().zip(identities) {
        scanner.seen_records.clear();
        let scan = scanner.user(user, logs);
        let mut user_tally = Tally::default();
        for (tool, tally) in &scan.by_tool {
            user_tally.merge(tally);
            by_tool.entry(tool).or_default().merge(tally);
        }
        total.merge(&user_tally);
        quotes.extend(scan.quotes);
        names.merge(scan.names);
        if user_tally.counts.sessions > 0 {
            by_user.push(UserStats {
                name: pseudonyms[identity].clone(),
                tally: user_tally,
            });
        }
    }

    // Slurs stay a host-wide total. Tied to a pseudonym or to the only person
    // using a tool, a count could point at someone on a small host.
    for user in &mut by_user {
        user.tally.phrases.remove(&Cat::Slur);
    }
    for tally in by_tool.values_mut() {
        tally.phrases.remove(&Cat::Slur);
    }

    // Report order must not follow passwd order, or the first entry would
    // always be root.
    by_user.sort_by(|a, b| a.name.cmp(&b.name));

    FunReport {
        generated_at: Utc::now(),
        scope,
        coverage: scanner.coverage,
        total,
        by_tool,
        by_user,
        quotes: pick_quotes(quotes),
        names: names.roasts(MAX_ROASTS),
    }
}

/// Local logs are always included. Remote snapshots are private and removed
/// when this function returns, including on collection errors.
pub async fn scan_hosts(
    range: Range,
    salt: u128,
    hosts: &[String],
    key: Option<&Path>,
    timeout: std::time::Duration,
) -> anyhow::Result<FunReport> {
    let snapshots = remote::collect(hosts, key, timeout).await?;
    let hosts_scanned = 1 + snapshots.len() as u64;
    let hosts_all_users =
        u64::from(discover::is_root()) + snapshots.iter().filter(|s| s.all_users).count() as u64;
    let scope = if hosts_all_users == hosts_scanned {
        Scope::AllUsers
    } else if hosts_all_users == 0 {
        Scope::CurrentUser
    } else {
        Scope::MixedUsers
    };
    let own_home = dirs::home_dir();
    let mut users: Vec<(HostUser, LogSet)> = discover::users_to_scan()
        .into_iter()
        .map(|user| {
            let trusted = own_home.as_ref() == Some(&user.home);
            let logs = discover::find_logs(&user, trusted);
            (user, logs)
        })
        .filter(|(_, logs)| !logs.is_empty())
        .collect();
    let mut identities: Vec<String> = users.iter().map(|(user, _)| user.name.clone()).collect();
    for snapshot in &snapshots {
        for (user, logs) in &snapshot.users {
            if !logs.is_empty() {
                // Namespacing keeps two accounts named "root" separate while
                // NameBook still sees their actual account/home for privacy.
                identities.push(format!("ssh:{}\0{}", snapshot.host, user.name));
                users.push((user.clone(), logs.clone()));
            }
        }
    }
    let mut report = build_report_with_ids(&users, &identities, scope, range, salt);
    report.coverage.hosts_scanned = hosts_scanned;
    report.coverage.hosts_all_users = hosts_all_users;
    report.coverage.files_unreadable += snapshots.iter().map(|s| s.unreadable).sum::<u64>();
    Ok(report)
}

/// Where the pseudonym salt lives, separate from the report so it never ends
/// up in shared output.
pub fn salt_path() -> anyhow::Result<PathBuf> {
    Ok(crate::paths::cache_dir()?.join("fun-stats.salt"))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ai-stats-fun-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn all_time() -> Range {
        Range {
            since: DateTime::from_timestamp(0, 0).unwrap(),
            until: DateTime::from_timestamp(4_000_000_000, 0).unwrap(),
        }
    }

    fn user(name: &str) -> HostUser {
        HostUser {
            name: name.to_string(),
            uid: None,
            home: PathBuf::from("/nonexistent"),
        }
    }

    fn claude_prompt(uuid: &str, at: &str, text: &str) -> String {
        serde_json::json!({"type": "user", "uuid": uuid, "timestamp": at,
                           "message": {"content": text}})
        .to_string()
    }

    fn write_log(path: &Path, lines: &[String]) {
        fs::write(path, lines.join("\n") + "\n").unwrap();
    }

    #[test]
    fn pseudonyms_are_stable_unique_and_salted() {
        let names: Vec<String> = (0..200).map(|i| format!("user{i}")).collect();
        let first = pseudonyms(&names, 7);
        assert_eq!(first, pseudonyms(&names, 7));
        assert_eq!(first.values().collect::<HashSet<_>>().len(), names.len());
        assert_ne!(first, pseudonyms(&names, 8));
    }

    #[test]
    fn a_copied_session_is_counted_once() {
        let dir = temp_dir("copied");
        let lines = [claude_prompt("u1", "2026-10-01T10:00:00Z", "hello there")];
        write_log(&dir.join("a.jsonl"), &lines);
        write_log(&dir.join("b.jsonl"), &lines);
        let logs = LogSet {
            claude_sessions: vec![dir.join("a.jsonl"), dir.join("b.jsonl")],
            ..LogSet::default()
        };
        let report = build_report(&[(user("a"), logs)], Scope::CurrentUser, all_time(), 1);
        assert_eq!(report.total.counts.prompts, 1);
    }

    #[test]
    fn accounts_on_different_hosts_remain_distinct_and_private() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        write_log(
            &path,
            &[claude_prompt(
                "p1",
                "2026-10-01T10:00:00Z",
                "private-secret-text",
            )],
        );
        let logs = LogSet {
            claude_sessions: vec![path],
            ..LogSet::default()
        };
        let users = [(user("root"), logs.clone()), (user("root"), logs)];
        let identities = [
            "root".to_string(),
            "ssh:root@remote.example\0root".to_string(),
        ];
        let report = build_report_with_ids(&users, &identities, Scope::AllUsers, all_time(), 42);
        assert_eq!(report.by_user.len(), 2);
        assert_ne!(report.by_user[0].name, report.by_user[1].name);
        assert_eq!(report.total.counts.prompts, 2);
        assert_eq!(report.by_tool["claude"].counts.prompts, 2);
        let reversed = build_report_with_ids(
            &[users[1].clone(), users[0].clone()],
            &[identities[1].clone(), identities[0].clone()],
            Scope::AllUsers,
            all_time(),
            42,
        );
        assert_eq!(
            report.by_user.iter().map(|u| &u.name).collect::<Vec<_>>(),
            reversed.by_user.iter().map(|u| &u.name).collect::<Vec<_>>()
        );
        let rendered = crate::report::fun::render_json(&report).unwrap();
        for private in [
            "root",
            "remote.example",
            "private-secret-text",
            "/nonexistent",
        ] {
            assert!(!rendered.contains(private));
        }
    }

    #[test]
    fn period_filter_applies_to_prompts_and_usage() {
        let dir = temp_dir("period");
        let usage = serde_json::json!({"input_tokens": 5, "output_tokens": 5});
        write_log(
            &dir.join("s.jsonl"),
            &[
                claude_prompt("u1", "2026-01-01T10:00:00Z", "old prompt"),
                serde_json::json!({"type": "assistant", "uuid": "a1",
                    "timestamp": "2026-01-01T10:00:01Z",
                    "message": {"id": "m1", "model": "x", "usage": usage, "content": []}})
                .to_string(),
                claude_prompt("u2", "2026-10-01T10:00:00Z", "new prompt"),
            ],
        );
        let logs = LogSet {
            claude_sessions: vec![dir.join("s.jsonl")],
            ..LogSet::default()
        };
        let range = Range {
            since: "2026-06-01T00:00:00Z".parse().unwrap(),
            until: "2026-12-01T00:00:00Z".parse().unwrap(),
        };
        let report = build_report(&[(user("a"), logs)], Scope::CurrentUser, range, 1);
        assert_eq!(report.total.counts.prompts, 1);
        assert_eq!(report.total.tokens(), 0);
    }

    #[test]
    fn history_only_fills_in_sessions_without_a_transcript() {
        let dir = temp_dir("history");
        write_log(
            &dir.join("known.jsonl"),
            &[claude_prompt(
                "u1",
                "2026-10-01T10:00:00Z",
                "from transcript",
            )],
        );
        write_log(
            &dir.join("history.jsonl"),
            &[
                serde_json::json!({"display": "same prompt again", "sessionId": "known",
                                   "timestamp": 1_790_000_000_000_i64})
                .to_string(),
                serde_json::json!({"display": "from a deleted session", "sessionId": "gone",
                                   "timestamp": 1_790_000_000_000_i64})
                .to_string(),
            ],
        );
        let logs = LogSet {
            claude_sessions: vec![dir.join("known.jsonl")],
            claude_history: Some(dir.join("history.jsonl")),
            ..LogSet::default()
        };
        let report = build_report(&[(user("a"), logs)], Scope::CurrentUser, all_time(), 1);
        assert_eq!(report.total.counts.prompts, 2);
        assert_eq!(report.total.counts.sessions, 2);
    }

    #[test]
    fn stats_are_kept_per_tool_and_per_user() {
        let dir = temp_dir("split");
        write_log(
            &dir.join("c.jsonl"),
            &[claude_prompt("u1", "2026-10-01T10:00:00Z", "one")],
        );
        write_log(
            &dir.join("x.jsonl"),
            &[
                serde_json::json!({"type": "event_msg", "timestamp": "2026-10-01T10:00:00Z",
                "payload": {"type": "item_completed", "item": {"type": "UserMessage", "id": "i1",
                    "content": [{"type": "text", "text": "two"}]}}})
                .to_string(),
            ],
        );
        let claude_logs = LogSet {
            claude_sessions: vec![dir.join("c.jsonl")],
            ..LogSet::default()
        };
        let codex_logs = LogSet {
            codex_sessions: vec![dir.join("x.jsonl")],
            ..LogSet::default()
        };
        let report = build_report(
            &[(user("a"), claude_logs), (user("b"), codex_logs)],
            Scope::AllUsers,
            all_time(),
            1,
        );
        assert_eq!(report.total.counts.prompts, 2);
        assert_eq!(report.by_tool["claude"].counts.prompts, 1);
        assert_eq!(report.by_tool["codex"].counts.prompts, 1);
        assert_eq!(report.by_user.len(), 2);
    }

    #[test]
    fn malformed_records_and_missing_files_are_counted_not_fatal() {
        let dir = temp_dir("broken");
        fs::write(dir.join("s.jsonl"), "not json\n{}\n").unwrap();
        let logs = LogSet {
            claude_sessions: vec![dir.join("s.jsonl"), dir.join("missing.jsonl")],
            ..LogSet::default()
        };
        let report = build_report(&[(user("a"), logs)], Scope::CurrentUser, all_time(), 1);
        assert_eq!(report.coverage.records_malformed, 1);
        assert_eq!(report.coverage.files_read, 1);
        assert_eq!(report.coverage.files_unreadable, 1);
    }

    #[test]
    fn quotes_rank_by_spice_and_drop_duplicates() {
        let quote = |words: &[&'static str], spice| QuoteCandidate {
            words: words.to_vec(),
            spice,
        };
        let picked = pick_quotes(vec![
            quote(&["nice", "work", "there"], 1),
            quote(&["this", "is", "wrong"], 3),
            quote(&["this", "is", "wrong"], 3),
        ]);
        assert_eq!(picked, ["this is wrong", "nice work there"]);
    }
}
