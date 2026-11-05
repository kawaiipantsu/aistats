//! Roasts project and domain names without printing them.
//!
//! A name is judged where it is found and only the verdict is kept: a masked
//! form built from its first letter, plus built-in strings such as the TLD and
//! the joke. The name itself is never stored. Results are host-wide, because a
//! domain tied to a pseudonym would say who works on what.

use std::collections::{BTreeMap, HashMap};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;

use serde::Serialize;

/// What stands in for the hidden part of a name. A fixed width hides the
/// length unless the joke is about the length.
const MASK: &str = "••••";

/// Names shorter than this show no letters at all: one letter of a five-letter
/// name gives away too much.
const MIN_LEN_FOR_INITIAL: usize = 6;

const LONG_NAME: usize = 16;
const VERY_LONG_NAME: usize = 24;
const MANY_SEPARATORS: usize = 3;
const MANY_DIGITS: usize = 3;

/// Top-level directories that are places, not projects.
const NOT_PROJECTS: &[&str] = &[
    "bin",
    "boot",
    "dev",
    "etc",
    "home",
    "lib",
    "log",
    "logs",
    "media",
    "mnt",
    "opt",
    "proc",
    "root",
    "run",
    "srv",
    "sys",
    "usr",
    "var",
    "workspace",
    "workspaces",
    "projects",
    "src",
];

/// Words that say nothing about a project, with what to say about them. Being
/// built in, these are the only name parts that may be printed in full.
const GENERIC: &[(&str, &str)] = &[
    ("test", "it went to production, didn't it"),
    ("testing", "it went to production, didn't it"),
    ("tmp", "nothing is more permanent than a temporary folder"),
    ("temp", "nothing is more permanent than a temporary folder"),
    ("new", "it stopped being new some time ago"),
    ("old", "and yet, still running"),
    ("final", "narrator: it was not the final one"),
    ("fixed", "narrator: it was not fixed"),
    ("real", "as opposed to all those fake ones"),
    ("backup", "the only backup, probably"),
    ("copy", "copy of copy of copy"),
    ("foo", "a classic of the genre"),
    ("bar", "a classic of the genre"),
    ("baz", "a classic of the genre"),
    ("asdf", "named by a cat walking across the keyboard"),
    ("qwerty", "named by a cat walking across the keyboard"),
    ("untitled", "naming things is hard"),
    ("misc", "where projects go to die"),
    ("stuff", "where projects go to die"),
    ("things", "where projects go to die"),
    ("project", "a project called project. Bold"),
    ("app", "an app called app. Bold"),
    ("my", "yes, we can tell it's yours"),
    ("demo", "the demo that never ended"),
    ("hello", "hello world, goodbye sleep"),
    ("world", "hello world, goodbye sleep"),
    ("wip", "work in progress, forever"),
    ("draft", "work in progress, forever"),
    ("scratch", "work in progress, forever"),
    ("sandbox", "work in progress, forever"),
    ("playground", "work in progress, forever"),
    ("experiment", "work in progress, forever"),
    ("v", "version numbers are a cry for help"),
];

const PUBLIC_DOMAINS: &[&str] = &[
    "amazon",
    "amazonaws",
    "anthropic",
    "apple",
    "bitbucket",
    "chatgpt",
    "claude",
    "cloudflare",
    "crates",
    "debian",
    "discord",
    "docker",
    "example",
    "facebook",
    "github",
    "gitlab",
    "gmail",
    "google",
    "hetzner",
    "instagram",
    "kernel",
    "linkedin",
    "linux",
    "microsoft",
    "mozilla",
    "npmjs",
    "openai",
    "outlook",
    "pypi",
    "python",
    "reddit",
    "rust-lang",
    "stackoverflow",
    "twitter",
    "ubuntu",
    "wikipedia",
    "youtube",
];

/// Suffixes matched wherever they appear, with what to say about each.
const TLDS: &[(&str, &str)] = &[
    ("com", ""),
    ("org", "a non-profit, in every sense"),
    ("net", ""),
    ("dk", "hygge-certified"),
    ("de", "gründlich, at least"),
    ("uk", ""),
    ("eu", "GDPR banner sold separately"),
    ("nl", ""),
    ("fr", ""),
    ("se", "assembly instructions not included"),
    ("xyz", "definitely not a phishing site"),
    ("lol", "the TLD is doing the comedy so you don't have to"),
    ("wtf", "the TLD asks the question we all had"),
    ("ninja", "nothing says stealth like a .ninja domain"),
    ("rocks", "it does not, in fact, rock"),
    ("guru", "self-appointed"),
    ("biz", "greetings from 2003"),
    ("club", "membership: you"),
];

/// Suffixes that are also common in code (`logger.info`, `self.app`), so they
/// only count next to a scheme, `www.`, or an `@`.
const CONTEXT_TLDS: &[(&str, &str)] = &[
    ("io", "seed round pending"),
    ("ai", "it's an if statement with a marketing budget"),
    ("dev", ""),
    ("app", ""),
    ("co", "couldn't afford the .com"),
    ("me", "a domain all about you"),
    ("no", ""),
    ("info", "greetings from 2003"),
    ("site", "as opposed to a sight"),
    ("online", "as opposed to offline"),
    ("top", "it is not"),
    ("live", "for now"),
    ("shop", ""),
    ("store", ""),
    ("blog", "last post: 2019"),
    ("space", ""),
    ("tech", "very tech"),
    ("cloud", "someone else's computer"),
    ("fun", "the judges disagree"),
];

/// `(second level, top level, label)` for suffixes spanning two labels.
const TWO_PART_TLDS: &[(&str, &str, &str)] = &[
    ("co", "uk", "co.uk"),
    ("org", "uk", "org.uk"),
    ("com", "au", "com.au"),
];

/// One name's verdict. `name` is already masked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Roast {
    pub name: String,
    pub joke: &'static str,
    #[serde(skip)]
    score: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct NameRoasts {
    pub projects_seen: u64,
    pub domains_seen: u64,
    pub tlds: BTreeMap<&'static str, u64>,
    pub projects: Vec<Roast>,
    pub domains: Vec<Roast>,
}

fn fingerprint(name: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    name.hash(&mut hasher);
    hasher.finish()
}

fn mask(name: &str) -> String {
    if name.chars().count() >= MIN_LEN_FOR_INITIAL {
        let initial: String = name.chars().take(1).collect();
        format!("{initial}{MASK}")
    } else {
        MASK.to_string()
    }
}

fn generic_joke(word: &str) -> Option<(&'static str, &'static str)> {
    GENERIC
        .iter()
        .find(|(generic, _)| *generic == word)
        .copied()
}

/// Rebuilds a token from a built-in word and up to three digits, or `None` if
/// it is anything else.
fn generic_token(token: &str) -> Option<(String, Option<&'static str>)> {
    let base = token.trim_end_matches(|c: char| c.is_ascii_digit());
    let digits = &token[base.len()..];
    if digits.len() > MANY_DIGITS || (base.is_empty() && digits.is_empty()) {
        return None;
    }
    if base.is_empty() {
        return Some((digits.to_string(), None));
    }
    let (word, joke) = generic_joke(base)?;
    Some((format!("{word}{digits}"), Some(joke)))
}

fn tokens(name: &str) -> Vec<&str> {
    name.split(['-', '_', '.', ' '])
        .filter(|token| !token.is_empty())
        .collect()
}

fn has_vowels(name: &str) -> bool {
    name.contains(['a', 'e', 'i', 'o', 'u', 'y'])
}

/// The worst thing that can be said about the shape of a name, as
/// `(score, joke, printed length)`.
fn shape_joke(name: &str) -> Option<(u32, &'static str, bool)> {
    let length = name.chars().count();
    let separators = name.matches(['-', '_']).count();
    let digits = name.chars().filter(char::is_ascii_digit).count();
    let letters = name.chars().filter(|c| c.is_alphabetic()).count();

    if length >= VERY_LONG_NAME {
        Some((30, "longer than its README", true))
    } else if letters >= 3 && !has_vowels(name) {
        Some((28, "vowels were not in the budget", false))
    } else if separators >= MANY_SEPARATORS {
        Some((26, "brought to you by the hyphen key", false))
    } else if digits >= MANY_DIGITS {
        Some((24, "sounds like a wifi password", false))
    } else if length <= 2 {
        Some((22, "minimalism has gone too far", false))
    } else if length >= LONG_NAME {
        Some((10, "a mouthful", true))
    } else {
        None
    }
}

fn roast_project(name: &str) -> Option<Roast> {
    let parts = tokens(name);
    let generic: Option<Vec<_>> = parts.iter().map(|t| generic_token(t)).collect();
    if let Some(generic) = generic.filter(|g| !g.is_empty()) {
        let joke = generic.iter().rev().find_map(|(_, joke)| *joke)?;
        let printed: Vec<&str> = generic.iter().map(|(word, _)| word.as_str()).collect();
        return Some(Roast {
            name: printed.join("-"),
            joke,
            score: 50,
        });
    }

    if let [private @ .., last] = parts.as_slice()
        && !private.is_empty()
        && let Some((word, Some(joke))) = generic_token(last)
    {
        return Some(Roast {
            name: format!("{}-{word}", mask(&private.join("-"))),
            joke,
            score: 40,
        });
    }

    shape_joke(name).map(|(score, joke, show_length)| Roast {
        name: if show_length {
            format!("{} ({} characters)", mask(name), name.chars().count())
        } else {
            mask(name)
        },
        joke,
        score,
    })
}

fn tld_entry(label: &str, in_url: bool) -> Option<(&'static str, &'static str)> {
    let found = TLDS.iter().find(|(tld, _)| *tld == label);
    let contextual = CONTEXT_TLDS.iter().find(|(tld, _)| in_url && *tld == label);
    found.or(contextual).copied()
}

/// Splits a host into its registrable name and suffix label, e.g.
/// `shop.example.co.uk` into `("example", "co.uk", joke)`.
fn registrable(host: &str, in_url: bool) -> Option<(&str, &'static str, &'static str)> {
    let labels: Vec<&str> = host.split('.').collect();
    let (last, rest) = labels.split_last()?;
    let (second, _) = rest.split_last()?;

    if let Some((_, _, suffix)) = TWO_PART_TLDS
        .iter()
        .find(|(sld, tld, _)| sld == second && tld == last)
    {
        let name = rest.iter().rev().nth(1)?;
        return Some((name, suffix, ""));
    }
    let (suffix, joke) = tld_entry(last, in_url)?;
    Some((second, suffix, joke))
}

/// Cuts one whitespace-delimited word down to a hostname, reporting whether it
/// looked like a URL or address rather than a bare `word.word`.
fn hostname(word: &str) -> Option<(String, bool)> {
    let word = word.trim_matches(|c: char| !c.is_alphanumeric());
    let (word, has_scheme) = match word.split_once("://") {
        Some((_, rest)) => (rest, true),
        None => (word, false),
    };
    let (word, has_at) = match word.rsplit_once('@') {
        Some((_, rest)) => (rest, true),
        None => (word, false),
    };
    let (word, has_www) = match word.strip_prefix("www.") {
        Some(rest) => (rest, true),
        None => (word, false),
    };

    let host: String = word
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
        .collect::<String>()
        .to_ascii_lowercase();
    let host = host.trim_matches(['.', '-']);
    let valid = host.contains('.')
        && host.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.len() <= 63
        });
    valid.then(|| (host.to_string(), has_scheme || has_at || has_www))
}

fn roast_domain(name: &str, suffix: &'static str, tld_joke: &'static str) -> Option<Roast> {
    let shape = shape_joke(name);
    let masked = format!("{}.{suffix}", mask(name));
    match shape {
        Some((score, joke, show_length)) if score > 10 || tld_joke.is_empty() => Some(Roast {
            name: if show_length {
                format!("{masked} ({} characters)", name.chars().count())
            } else {
                masked
            },
            joke,
            score,
        }),
        _ if !tld_joke.is_empty() => Some(Roast {
            name: masked,
            joke: tld_joke,
            score: 20,
        }),
        _ => None,
    }
}

/// Every project and domain seen for one user, by fingerprint.
#[derive(Debug, Default)]
pub struct NameBook {
    home: String,
    user: String,
    projects: HashMap<u64, Option<Roast>>,
    domains: HashMap<u64, (&'static str, Option<Roast>)>,
}

impl NameBook {
    pub fn for_user(home: &Path, user: &str) -> Self {
        Self {
            home: home.to_string_lossy().trim_end_matches('/').to_string(),
            user: user.to_ascii_lowercase(),
            ..Self::default()
        }
    }

    /// Records the project behind a working directory. A home directory's
    /// name is an account name, so it never counts.
    pub fn see_cwd(&mut self, cwd: &str) {
        let cwd = cwd.trim_end_matches('/');
        if cwd.is_empty() || cwd == self.home {
            return;
        }
        let Some(name) = cwd.rsplit('/').next() else {
            return;
        };
        let name = name.to_lowercase();
        if name.is_empty()
            || name == self.user
            || name.starts_with('.')
            || NOT_PROJECTS.contains(&name.as_str())
        {
            return;
        }
        self.projects
            .entry(fingerprint(&name))
            .or_insert_with(|| roast_project(&name));
    }

    /// Records every domain mentioned in a prompt.
    pub fn see_text(&mut self, text: &str) {
        for word in text.split_whitespace() {
            let Some((host, in_url)) = hostname(word) else {
                continue;
            };
            let Some((name, suffix, joke)) = registrable(&host, in_url) else {
                continue;
            };
            if name.len() < 2
                || name.chars().all(|c| c.is_ascii_digit())
                || PUBLIC_DOMAINS.contains(&name)
            {
                continue;
            }
            self.domains
                .entry(fingerprint(name))
                .or_insert_with(|| (suffix, roast_domain(name, suffix, joke)));
        }
    }

    pub fn merge(&mut self, other: Self) {
        self.projects.extend(other.projects);
        self.domains.extend(other.domains);
    }

    /// Picks the worst `top` of each kind. Order is fully determined by the
    /// verdicts, so runs over the same logs agree.
    pub fn roasts(&self, top: usize) -> NameRoasts {
        let mut tlds = BTreeMap::new();
        for (suffix, _) in self.domains.values() {
            *tlds.entry(*suffix).or_default() += 1;
        }
        let projects = self.projects.values().flatten().cloned();
        let domains = self.domains.values().filter_map(|(_, r)| r.clone());
        NameRoasts {
            projects_seen: self.projects.len() as u64,
            domains_seen: self.domains.len() as u64,
            tlds,
            projects: worst(projects, top),
            domains: worst(domains, top),
        }
    }
}

fn worst(roasts: impl Iterator<Item = Roast>, top: usize) -> Vec<Roast> {
    let mut roasts: Vec<Roast> = roasts.collect();
    roasts.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.name.cmp(&b.name)));
    roasts.dedup_by(|a, b| a.name == b.name);
    roasts.truncate(top);
    roasts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> NameBook {
        NameBook::for_user(Path::new("/home/alice"), "alice")
    }

    fn project(name: &str) -> Option<String> {
        roast_project(name).map(|roast| roast.name)
    }

    fn domains(text: &str) -> NameRoasts {
        let mut names = book();
        names.see_text(text);
        names.roasts(10)
    }

    #[test]
    fn fully_generic_names_are_printed_because_they_are_built_in() {
        assert_eq!(project("test2").as_deref(), Some("test2"));
        assert_eq!(project("new-final_v2").as_deref(), Some("new-final-v2"));
    }

    #[test]
    fn private_names_keep_only_an_initial_and_a_generic_suffix() {
        assert_eq!(project("falcon-final").as_deref(), Some("f••••-final"));
        assert_eq!(project("acme-v2").as_deref(), Some("••••-v2"));
    }

    #[test]
    fn unremarkable_names_get_no_roast() {
        assert_eq!(project("falcon"), None);
    }

    #[test]
    fn shape_jokes_reveal_at_most_an_initial_and_the_length() {
        assert_eq!(
            project("internal-customer-dashboard").as_deref(),
            Some("i•••• (27 characters)")
        );
        assert_eq!(project("brd-prt-sys-mgr").as_deref(), Some("b••••"));
        assert_eq!(project("x7").as_deref(), Some("••••"));
    }

    #[test]
    fn home_directories_and_system_folders_are_not_projects() {
        let mut names = book();
        names.see_cwd("/home/alice");
        names.see_cwd("/home/alice/");
        names.see_cwd("/srv/alice");
        names.see_cwd("/var/log");
        names.see_cwd("/");
        names.see_cwd("/home/alice/.config");
        assert_eq!(names.roasts(10).projects_seen, 0);
    }

    #[test]
    fn a_home_named_differently_from_its_account_is_still_not_a_project() {
        let mut names = NameBook::for_user(Path::new("/data/people/jsmith"), "john");
        names.see_cwd("/data/people/jsmith");
        assert_eq!(names.roasts(10).projects_seen, 0);
    }

    #[test]
    fn a_project_is_counted_once_however_often_it_appears() {
        let mut names = book();
        names.see_cwd("/srv/tmp");
        names.see_cwd("/srv/tmp");
        assert_eq!(names.roasts(10).projects_seen, 1);
    }

    #[test]
    fn domains_are_found_in_urls_addresses_and_bare_text() {
        let report = domains(
            "see https://www.secret-client.dk/path?x=1, mail me@falcon.xyz or try plain.dk.",
        );
        assert_eq!(report.domains_seen, 3);
        assert_eq!(report.tlds, BTreeMap::from([("dk", 2), ("xyz", 1)]));
    }

    #[test]
    fn subdomains_and_two_part_suffixes_reduce_to_the_registrable_name() {
        let report = domains("https://a.b.falcon.co.uk and falcon.co.uk");
        assert_eq!(report.domains_seen, 1);
        assert_eq!(report.tlds, BTreeMap::from([("co.uk", 1)]));
    }

    #[test]
    fn code_and_file_names_are_not_domains() {
        for text in [
            "call logger.info and self.app then os.path",
            "edit README.md and main.rs and setup.py and run.sh",
            "version 1.2.3 at 192.168.1.1",
            "user.name.first",
        ] {
            assert_eq!(domains(text).domains_seen, 0, "{text}");
        }
    }

    #[test]
    fn suffixes_used_in_code_count_inside_a_url() {
        assert_eq!(domains("https://falcon.io/x").tlds["io"], 1);
        assert_eq!(domains("falcon.io").domains_seen, 0);
    }

    #[test]
    fn public_sites_are_not_anyones_fault() {
        assert_eq!(
            domains("https://github.com/x and www.google.com").domains_seen,
            0
        );
    }

    #[test]
    fn domain_roasts_never_contain_the_name() {
        let report = domains("https://very-private-client-name-here.xyz and falcon.lol");
        for roast in &report.domains {
            assert!(!roast.name.contains("private"), "{}", roast.name);
            assert!(!roast.name.contains("falcon"), "{}", roast.name);
        }
        assert_eq!(report.domains.len(), 2);
    }

    #[test]
    fn short_domains_show_no_letters() {
        let report = domains("foo.xyz");
        assert_eq!(report.domains[0].name, "••••.xyz");
    }

    #[test]
    fn worst_names_rank_first_and_output_is_stable() {
        let mut names = book();
        names.see_cwd("/srv/a-very-long-project-name-goes-here");
        names.see_cwd("/srv/test");
        let first = names.roasts(1);
        assert_eq!(first.projects[0].name, "test");
        assert_eq!(first, names.roasts(1));
    }
}
