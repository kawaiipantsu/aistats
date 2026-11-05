//! Turns [`Event`]s into counters.
//!
//! [`Tally`] is the privacy boundary of `--fun-stats`: it holds numbers,
//! dates, emoji characters, and `&'static str` labels from
//! [`super::lexicon`], and has no field that could carry a string read from a
//! log. A leak would have to change these types, not just a format string.

use std::collections::BTreeMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use chrono::{DateTime, Datelike, Local, NaiveDate, Timelike, Utc};
use serde::Serialize;

use super::lexicon::{Cat, Lexicon, known_command, own_brand};
use super::names::NameBook;
use super::parse::Event;

/// Characters of a prompt that are analyzed. Beyond this it is a paste, not
/// something the person typed, and a pasted log would otherwise pollute the
/// phrase counts.
const MAX_PROMPT_BYTES: usize = 4 * 1024;
const MAX_AI_TEXT_BYTES: usize = 64 * 1024;

/// A gap longer than this means the person walked away, so it does not count
/// towards how long a session lasted.
const IDLE_GAP_SECS: i64 = 30 * 60;

/// Shortest word worth a record, and the longest a real word can be: anything
/// beyond `pneumonoultramicroscopicsilicovolcanoconiosis` is a blob.
const MIN_RECORD_WORD: usize = 8;
const MAX_REAL_WORD: usize = 45;

const QUOTE_MIN_WORDS: usize = 3;
const QUOTE_MAX_WORDS: usize = 12;

macro_rules! counters {
    (sum { $($sum:ident),* $(,)? } max { $($max:ident),* $(,)? }) => {
        #[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
        pub struct Counters {
            $(pub $sum: u64,)*
            $(pub $max: u64,)*
        }

        impl Counters {
            fn merge(&mut self, other: &Self) {
                $(self.$sum += other.$sum;)*
                $(self.$max = self.$max.max(other.$max);)*
            }
        }
    };
}

counters! {
    sum {
        sessions,
        prompts,
        prompt_chars,
        prompt_words,
        caps_prompts,
        question_prompts,
        exclamations,
        repeated_prompts,
        ai_messages,
        ai_words,
        ai_em_dashes,
        ai_exclamations,
        tool_calls,
        tool_failures,
        tool_denials,
        interrupts,
        compactions,
        refusals,
        api_errors,
        claims_busted,
        ai_sleep_secs,
        git_commits,
        input_tokens,
        cached_tokens,
        output_tokens,
    }
    max {
        longest_prompt_chars,
        longest_tool_run,
        longest_meltdown,
        longest_session_secs,
        longest_punctuation_run,
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Tally {
    #[serde(flatten)]
    pub counts: Counters,
    pub by_hour: [u64; 24],
    /// Monday first.
    pub by_weekday: [u64; 7],
    pub by_day: BTreeMap<NaiveDate, u64>,
    pub phrases: BTreeMap<Cat, BTreeMap<&'static str, u64>>,
    pub rubber_stamps: BTreeMap<&'static str, u64>,
    pub happy_smileys: BTreeMap<&'static str, u64>,
    pub sad_smileys: BTreeMap<&'static str, u64>,
    pub human_emoji: BTreeMap<char, u64>,
    pub ai_emoji: BTreeMap<char, u64>,
    pub tools: BTreeMap<&'static str, u64>,
    pub commands: BTreeMap<&'static str, u64>,
    pub danger: BTreeMap<&'static str, u64>,
    pub models: BTreeMap<&'static str, u64>,
    /// Topics mentioned in the same prompt as swearing or frustration.
    pub heated_topics: BTreeMap<&'static str, u64>,
    /// Bursts such as `!!!` and `...`, by kind.
    pub punctuation: BTreeMap<&'static str, u64>,
    /// Length and first letter. The word itself may be a private name.
    pub longest_word: Option<(u64, char)>,
}

fn merge_map<K: Ord + Clone>(into: &mut BTreeMap<K, u64>, from: &BTreeMap<K, u64>) {
    for (key, count) in from {
        *into.entry(key.clone()).or_default() += count;
    }
}

impl Tally {
    pub fn merge(&mut self, other: &Self) {
        self.counts.merge(&other.counts);
        for (mine, theirs) in self.by_hour.iter_mut().zip(other.by_hour) {
            *mine += theirs;
        }
        for (mine, theirs) in self.by_weekday.iter_mut().zip(other.by_weekday) {
            *mine += theirs;
        }
        merge_map(&mut self.by_day, &other.by_day);
        for (cat, labels) in &other.phrases {
            merge_map(self.phrases.entry(*cat).or_default(), labels);
        }
        merge_map(&mut self.rubber_stamps, &other.rubber_stamps);
        merge_map(&mut self.happy_smileys, &other.happy_smileys);
        merge_map(&mut self.sad_smileys, &other.sad_smileys);
        merge_map(&mut self.human_emoji, &other.human_emoji);
        merge_map(&mut self.ai_emoji, &other.ai_emoji);
        merge_map(&mut self.tools, &other.tools);
        merge_map(&mut self.commands, &other.commands);
        merge_map(&mut self.danger, &other.danger);
        merge_map(&mut self.models, &other.models);
        merge_map(&mut self.heated_topics, &other.heated_topics);
        merge_map(&mut self.punctuation, &other.punctuation);
        self.longest_word = self.longest_word.max(other.longest_word);
    }

    /// Total hits across every label in a category.
    pub fn total(&self, cat: Cat) -> u64 {
        self.phrases
            .get(&cat)
            .map_or(0, |labels| labels.values().sum())
    }

    pub fn tokens(&self) -> u64 {
        self.counts.input_tokens + self.counts.cached_tokens + self.counts.output_tokens
    }

    fn phrase(&mut self, cat: Cat, label: &'static str) {
        *self
            .phrases
            .entry(cat)
            .or_default()
            .entry(label)
            .or_default() += 1;
    }
}

/// A prompt that qualified for quoting. `words` are built-in strings, not log
/// text; see [`Lexicon::quote`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuoteCandidate {
    pub words: Vec<&'static str>,
    pub spice: u32,
}

/// Lowercases and splits on anything that is not a letter or digit, dropping
/// apostrophes so "don't" and "dont" are the same token.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for c in text.chars() {
        if c == '\'' || c == '’' {
            continue;
        }
        if c.is_alphanumeric() {
            current.extend(c.to_lowercase());
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn strip_blocks(text: &str, open: &str, close: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(open) {
        out.push_str(&rest[..start]);
        let body = &rest[start + open.len()..];
        rest = body
            .find(close)
            .map_or("", |end| &body[end + close.len()..]);
    }
    out.push_str(rest);
    out
}

fn truncate_at_char(text: &mut String, max: usize) {
    if text.len() > max {
        let mut end = max;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
}

/// Removes pasted material and code so that what remains is prose: an error
/// message in a pasted stack trace is not the person shouting.
fn prose(text: &str, max: usize) -> String {
    let text = strip_blocks(text, "<pasted_content", "</pasted_content>");
    let text = strip_blocks(&text, "<system-reminder>", "</system-reminder>");
    let mut text = strip_blocks(&text, "```", "```");
    truncate_at_char(&mut text, max);
    text
}

/// Dingbats with emoji presentation. The rest of that block is typography,
/// and a pasted terminal's `❯` is not a mood.
const EMOJI_DINGBATS: &[u32] = &[
    0x2702, 0x2705, 0x2708, 0x2709, 0x270A, 0x270B, 0x270C, 0x270D, 0x270F, 0x2712, 0x2714, 0x2716,
    0x2728, 0x2733, 0x2734, 0x2744, 0x2747, 0x274C, 0x274E, 0x2753, 0x2754, 0x2755, 0x2757, 0x2763,
    0x2764, 0x2795, 0x2796, 0x2797, 0x27A1, 0x27B0, 0x27BF,
];

/// Plain symbols that sit inside the emoji ranges, such as ★ and ☐.
const NOT_EMOJI: &[u32] = &[0x2605, 0x2606, 0x2610, 0x2612, 0x261E, 0x266A, 0x266B];

/// Finds bursts of `!`, `?`, and `.`, as `(kind, length)`. Two dots is a path
/// and three is an ellipsis, so dots need three while the others need two.
fn punctuation_runs(text: &str) -> Vec<(&'static str, u64)> {
    let mut runs = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if !matches!(c, '!' | '?' | '.') {
            continue;
        }
        let (mut length, mut bang, mut question) = (1, c == '!', c == '?');
        while let Some(next) = chars.next_if(|next| matches!(next, '!' | '?' | '.')) {
            length += 1;
            bang |= next == '!';
            question |= next == '?';
        }
        let kind = match (bang, question) {
            (true, true) => "?!",
            (true, false) => "!!!",
            (false, true) => "???",
            (false, false) => "...",
        };
        if length >= if kind == "..." { 3 } else { 2 } {
            runs.push((kind, length));
        }
    }
    runs
}

/// A word's length and initial, if it looks like an actual word: letters only,
/// no inner capitals (`camelCase`), and vowels in natural proportion, which
/// rules out hashes and base64.
fn real_word(raw: &str) -> Option<(u64, char)> {
    let word = raw.trim_matches(|c: char| !c.is_alphanumeric());
    let length = word.chars().count();
    if !(MIN_RECORD_WORD..=MAX_REAL_WORD).contains(&length)
        || !word.chars().all(char::is_alphabetic)
        || word.chars().skip(1).any(char::is_uppercase)
    {
        return None;
    }
    let vowels = word
        .to_lowercase()
        .chars()
        .filter(|c| "aeiouyæøåäöü".contains(*c))
        .count();
    if vowels * 5 < length || vowels * 10 > length * 7 {
        return None;
    }
    word.to_lowercase()
        .chars()
        .next()
        .map(|c| (length as u64, c))
}

fn is_emoji(c: char) -> bool {
    let code = c as u32;
    match code {
        0x2600..=0x26FF => !NOT_EMOJI.contains(&code),
        0x2700..=0x27BF => EMOJI_DINGBATS.contains(&code),
        0x1F300..=0x1F3FA
        | 0x1F400..=0x1F64F
        | 0x1F680..=0x1F6FF
        | 0x1F900..=0x1F9FF
        | 0x1FA70..=0x1FAFF => true,
        _ => false,
    }
}

fn count_emoji(text: &str, into: &mut BTreeMap<char, u64>) {
    for c in text.chars().filter(|c| is_emoji(*c)) {
        *into.entry(c).or_default() += 1;
    }
}

/// True when nearly all letters are capitals and there are enough of them
/// that it is shouting rather than an acronym.
fn is_shouting(text: &str) -> bool {
    let (mut letters, mut upper) = (0u32, 0u32);
    for c in text.chars().filter(|c| c.is_alphabetic()) {
        letters += 1;
        upper += u32::from(c.is_uppercase());
    }
    letters >= 8 && upper * 10 >= letters * 8
}

fn heredoc_terminator(line: &str) -> Option<String> {
    let rest = &line[line.find("<<")? + 2..];
    if rest.starts_with('<') {
        return None; // A here-string has no body.
    }
    let rest = rest.strip_prefix('-').unwrap_or(rest).trim_start();
    let word: String = rest
        .trim_start_matches(['\'', '"'])
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    word.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        .then_some(word)
}

/// Drops heredoc bodies. Around a third of real commands carry one, holding a
/// file being written or a Python script, and a line of that which happens to
/// start with `rm` was never run as a command.
fn without_heredocs(script: &str) -> String {
    let mut out = String::with_capacity(script.len());
    let mut terminator: Option<String> = None;
    for line in script.lines() {
        match &terminator {
            Some(end) => {
                if line.trim() == end {
                    terminator = None;
                }
            }
            None => {
                out.push_str(line);
                out.push('\n');
                terminator = heredoc_terminator(line);
            }
        }
    }
    out
}

/// Splits a shell script into pipelines, then each pipeline into stages.
fn pipelines(script: &str) -> impl Iterator<Item = Vec<&str>> {
    script
        .split(['\n', ';'])
        .flat_map(|part| part.split("&&"))
        .flat_map(|part| part.split("||"))
        .map(|pipeline| pipeline.split('|').collect())
}

/// The words of one pipeline stage, without leading `sudo` and `VAR=value`.
fn stage_words(stage: &str) -> (bool, Vec<&str>) {
    let mut words: Vec<&str> = stage.split_whitespace().collect();
    let mut sudo = false;
    loop {
        match words.first() {
            Some(&"sudo") => {
                sudo = true;
                words.remove(0);
            }
            Some(word) if word.contains('=') && !word.starts_with('-') => {
                words.remove(0);
            }
            _ => return (sudo, words),
        }
    }
}

fn is_recursive_force(words: &[&str]) -> bool {
    let (mut recursive, mut force) = (false, false);
    for word in words.iter().filter(|word| word.starts_with('-')) {
        if *word == "--recursive" {
            recursive = true;
        } else if *word == "--force" {
            force = true;
        } else if !word.starts_with("--") {
            recursive |= word.contains(['r', 'R']);
            force |= word.contains('f');
        }
    }
    recursive && force
}

fn add(map: &mut BTreeMap<&'static str, u64>, label: &'static str) {
    *map.entry(label).or_default() += 1;
}

fn record_command(tally: &mut Tally, script: &str) {
    let script = without_heredocs(script);
    let lower = script.to_ascii_lowercase();
    if ["drop table", "drop database", "truncate table"]
        .iter()
        .any(|needle| lower.contains(needle))
    {
        add(&mut tally.danger, "DROP TABLE");
    }
    if script.contains("2>/dev/null") || script.contains("2> /dev/null") {
        add(&mut tally.danger, "2>/dev/null");
    }
    if script.contains("|| true") {
        add(&mut tally.danger, "|| true");
    }
    if script.contains("--no-verify") {
        add(&mut tally.danger, "--no-verify");
    }

    for stages in pipelines(&script) {
        let parsed: Vec<_> = stages.iter().map(|stage| stage_words(stage)).collect();
        for (index, (sudo, words)) in parsed.iter().enumerate() {
            if *sudo {
                add(&mut tally.danger, "sudo");
            }
            let Some(first) = words.first() else {
                continue;
            };
            let name = first.rsplit('/').next().unwrap_or(first);
            if let Some(known) = known_command(name) {
                add(&mut tally.commands, known);
            }
            let rest = &words[1..];
            let flagged = match name {
                "rm" if is_recursive_force(rest) => Some("rm -rf"),
                "git" => git_danger(tally, rest),
                "chmod" if rest.contains(&"777") => Some("chmod 777"),
                "kill" | "pkill" | "killall"
                    if rest
                        .iter()
                        .any(|w| matches!(*w, "-9" | "-KILL" | "-SIGKILL")) =>
                {
                    Some("kill -9")
                }
                "dd" if rest.iter().any(|w| w.starts_with("of=/dev/")) => Some("dd of=/dev/"),
                "reboot" | "shutdown" | "poweroff" => Some("reboot"),
                "docker" if rest.starts_with(&["system", "prune"]) => Some("docker system prune"),
                "curl" | "wget" => parsed.get(index + 1).and_then(|(_, next)| {
                    matches!(next.first(), Some(&("sh" | "bash" | "zsh"))).then_some("curl | bash")
                }),
                "sleep" => {
                    let secs = rest.first().and_then(|w| w.parse::<u64>().ok());
                    tally.counts.ai_sleep_secs += secs.unwrap_or(0);
                    None
                }
                _ if name.starts_with("mkfs") => Some("mkfs"),
                _ => None,
            };
            if let Some(label) = flagged {
                add(&mut tally.danger, label);
            }
        }
    }
}

fn git_danger(tally: &mut Tally, args: &[&str]) -> Option<&'static str> {
    match args.first() {
        Some(&"commit") => {
            tally.counts.git_commits += 1;
            None
        }
        Some(&"push")
            if args
                .iter()
                .any(|a| matches!(*a, "-f" | "--force" | "--force-with-lease")) =>
        {
            Some("git push --force")
        }
        Some(&"reset") if args.contains(&"--hard") => Some("git reset --hard"),
        Some(&"clean") if args.iter().any(|a| a.starts_with('-') && a.contains('f')) => {
            Some("git clean -f")
        }
        _ => None,
    }
}

/// Analyzes one session into a [`Tally`], keeping just enough state for the
/// stats that depend on message order.
pub struct SessionAnalyzer<'a> {
    lexicon: &'a Lexicon,
    tool: &'static str,
    tally: &'a mut Tally,
    quotes: &'a mut Vec<QuoteCandidate>,
    names: &'a mut NameBook,
    saw_event: bool,
    last_prompt: Option<u64>,
    meltdown: u64,
    tool_run: u64,
    claim_pending: bool,
    last_seen: Option<DateTime<Utc>>,
    active_secs: i64,
}

impl<'a> SessionAnalyzer<'a> {
    pub fn new(
        lexicon: &'a Lexicon,
        tool: &'static str,
        tally: &'a mut Tally,
        quotes: &'a mut Vec<QuoteCandidate>,
        names: &'a mut NameBook,
    ) -> Self {
        Self {
            lexicon,
            tool,
            tally,
            quotes,
            names,
            saw_event: false,
            last_prompt: None,
            meltdown: 0,
            tool_run: 0,
            claim_pending: false,
            last_seen: None,
            active_secs: 0,
        }
    }

    pub fn event(&mut self, at: Option<DateTime<Utc>>, event: &Event<'_>) {
        self.saw_event = true;
        if let Some(at) = at {
            self.clock(at);
        }
        let counts = &mut self.tally.counts;
        match event {
            Event::Cwd(path) => self.names.see_cwd(path),
            Event::Prompt(text) => self.prompt(text, at),
            Event::AiText(text) => self.ai_text(text),
            Event::Tool { label, command } => {
                counts.tool_calls += 1;
                self.tool_run += 1;
                counts.longest_tool_run = counts.longest_tool_run.max(self.tool_run);
                add(&mut self.tally.tools, label);
                if let Some(command) = command {
                    record_command(self.tally, command);
                }
            }
            Event::ToolFailed => counts.tool_failures += 1,
            Event::ToolDenied => counts.tool_denials += 1,
            Event::Interrupted => counts.interrupts += 1,
            Event::Compaction => counts.compactions += 1,
            Event::Refusal => counts.refusals += 1,
            Event::ApiError => counts.api_errors += 1,
            Event::Usage {
                model,
                input,
                cached,
                output,
            } => {
                counts.input_tokens += input;
                counts.cached_tokens += cached;
                counts.output_tokens += output;
                *self.tally.models.entry(model).or_default() += input + cached + output;
            }
        }
    }

    /// Closes the session. A file with nothing in the requested period is not
    /// counted as a session.
    pub fn finish(self) {
        if self.saw_event {
            let counts = &mut self.tally.counts;
            counts.sessions += 1;
            counts.longest_session_secs = counts.longest_session_secs.max(self.active_secs as u64);
        }
    }

    fn clock(&mut self, at: DateTime<Utc>) {
        if let Some(previous) = self.last_seen {
            let gap = (at - previous).num_seconds();
            if (0..=IDLE_GAP_SECS).contains(&gap) {
                self.active_secs += gap;
            }
        }
        self.last_seen = Some(at);
    }

    fn prompt(&mut self, raw: &str, at: Option<DateTime<Utc>>) {
        let text = prose(raw, MAX_PROMPT_BYTES);
        let tokens = tokenize(&text);
        if tokens.is_empty() {
            return;
        }

        let chars = raw.chars().count() as u64;
        let counts = &mut self.tally.counts;
        counts.prompts += 1;
        counts.prompt_chars += chars;
        counts.prompt_words += tokens.len() as u64;
        counts.longest_prompt_chars = counts.longest_prompt_chars.max(chars);
        counts.exclamations += text.matches('!').count() as u64;
        counts.caps_prompts += u64::from(is_shouting(&text));
        counts.question_prompts += u64::from(text.contains('?'));

        if let Some(at) = at {
            let local = at.with_timezone(&Local);
            self.tally.by_hour[local.hour() as usize] += 1;
            self.tally.by_weekday[local.weekday().num_days_from_monday() as usize] += 1;
            *self.tally.by_day.entry(local.date_naive()).or_default() += 1;
        }

        for word in text.split_whitespace() {
            if let Some((smiley, happy)) = self.lexicon.smiley(word) {
                let map = if happy {
                    &mut self.tally.happy_smileys
                } else {
                    &mut self.tally.sad_smileys
                };
                add(map, smiley);
            }
        }
        count_emoji(&text, &mut self.tally.human_emoji);
        self.names.see_text(&text);

        for (kind, length) in punctuation_runs(&text) {
            add(&mut self.tally.punctuation, kind);
            if kind != "..." {
                let longest = &mut self.tally.counts.longest_punctuation_run;
                *longest = (*longest).max(length);
            }
        }
        let longest_word = text.split_whitespace().filter_map(real_word).max();
        self.tally.longest_word = self.tally.longest_word.max(longest_word);

        let stamp = self.lexicon.rubber_stamp(&tokens);
        if let Some(label) = stamp {
            add(&mut self.tally.rubber_stamps, label);
        }

        let mut hits: Vec<(Cat, &'static str)> = Vec::new();
        self.lexicon.human.scan(&tokens, |cat, label| {
            hits.push((cat, label));
        });
        for token in &tokens {
            if token.starts_with("haha") {
                hits.push((Cat::Laughter, "haha"));
            } else if token.starts_with("hehe") {
                hits.push((Cat::Laughter, "hehe"));
            }
        }
        let own = own_brand(self.tool);
        hits.retain(|(cat, label)| *cat != Cat::Rival || !own.contains(label));
        // The exact tokens are matched above too, so "haha" alone is counted
        // only once.
        hits.sort_unstable();
        hits.dedup_by(|a, b| a.0 == Cat::Laughter && a == b);

        for (cat, label) in &hits {
            self.tally.phrase(*cat, label);
        }

        self.track_mood(&tokens, &hits, stamp.is_some());
        self.collect_quotes(&text);
    }

    /// Stats that depend on how a prompt relates to the conversation so far.
    fn track_mood(&mut self, tokens: &[String], hits: &[(Cat, &'static str)], stamp: bool) {
        let is_cat = |wanted: &[Cat]| hits.iter().any(|(cat, _)| wanted.contains(cat));
        let angry = is_cat(&[
            Cat::Swear,
            Cat::Abuse,
            Cat::Hate,
            Cat::Slur,
            Cat::Frustration,
        ]);
        let counts = &mut self.tally.counts;

        if angry {
            self.meltdown += 1;
            counts.longest_meltdown = counts.longest_meltdown.max(self.meltdown);
            for (_, topic) in hits.iter().filter(|(cat, _)| *cat == Cat::Topic) {
                add(&mut self.tally.heated_topics, topic);
            }
            if self.claim_pending {
                counts.claims_busted += 1;
            }
        } else {
            self.meltdown = 0;
        }
        self.claim_pending = false;
        self.tool_run = 0;

        let mut hasher = DefaultHasher::new();
        tokens.hash(&mut hasher);
        let fingerprint = hasher.finish();
        if !stamp && self.last_prompt == Some(fingerprint) {
            self.tally.counts.repeated_prompts += 1;
        }
        self.last_prompt = Some(fingerprint);
    }

    /// Considers each sentence of a prompt for quoting. The best lines tend
    /// to sit inside longer prompts, and judging sentences separately means
    /// one quotable line is not disqualified by the rest of the message.
    fn collect_quotes(&mut self, text: &str) {
        for sentence in text.split(['.', '!', '?', ';', '\n']) {
            let tokens = tokenize(sentence);
            if !(QUOTE_MIN_WORDS..=QUOTE_MAX_WORDS).contains(&tokens.len()) {
                continue;
            }
            let mut spice = 0;
            self.lexicon.human.scan(&tokens, |cat, _| {
                spice += match cat {
                    Cat::Swear | Cat::Abuse | Cat::Hate => 3,
                    Cat::Frustration | Cat::Begging | Cat::Threat | Cat::Existential => 2,
                    Cat::Praise => 1,
                    _ => 0,
                };
            });
            if spice > 0
                && let Some(words) = self.lexicon.quote(&tokens)
            {
                self.quotes.push(QuoteCandidate { words, spice });
            }
        }
    }

    fn ai_text(&mut self, raw: &str) {
        let text = prose(raw, MAX_AI_TEXT_BYTES);
        let tokens = tokenize(&text);
        if tokens.is_empty() {
            return;
        }
        let counts = &mut self.tally.counts;
        counts.ai_messages += 1;
        counts.ai_words += tokens.len() as u64;
        counts.ai_em_dashes += text.matches('—').count() as u64;
        counts.ai_exclamations += text.matches('!').count() as u64;
        count_emoji(&text, &mut self.tally.ai_emoji);

        let mut hits: Vec<(Cat, &'static str)> = Vec::new();
        self.lexicon.ai.scan(&tokens, |cat, label| {
            hits.push((cat, label));
        });
        for (cat, label) in hits {
            self.claim_pending |= cat == Cat::Claim;
            self.tally.phrase(cat, label);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::*;

    fn analyze(tool: &'static str, events: &[Event<'_>]) -> (Tally, Vec<QuoteCandidate>) {
        let lexicon = Lexicon::new();
        let mut tally = Tally::default();
        let mut quotes = Vec::new();
        let mut names = NameBook::default();
        let mut session = SessionAnalyzer::new(&lexicon, tool, &mut tally, &mut quotes, &mut names);
        for event in events {
            session.event(None, event);
        }
        session.finish();
        (tally, quotes)
    }

    fn prompt(text: &str) -> Event<'_> {
        Event::Prompt(Cow::Borrowed(text))
    }

    fn ai(text: &str) -> Event<'_> {
        Event::AiText(Cow::Borrowed(text))
    }

    fn run(script: &str) -> Tally {
        let mut tally = Tally::default();
        record_command(&mut tally, script);
        tally
    }

    #[test]
    fn counts_prompt_shape() {
        let (tally, _) = analyze(
            "claude",
            &[
                prompt("WHY IS THIS BROKEN???"),
                prompt("please fix the docker build!"),
                prompt("continue"),
            ],
        );
        assert_eq!(tally.counts.prompts, 3);
        assert_eq!(tally.counts.caps_prompts, 1);
        assert_eq!(tally.counts.question_prompts, 1);
        assert_eq!(tally.counts.exclamations, 1);
        assert_eq!(tally.rubber_stamps["continue"], 1);
        assert_eq!(tally.total(Cat::Polite), 1);
        assert_eq!(tally.total(Cat::Topic), 1);
    }

    #[test]
    fn pasted_material_and_code_are_not_the_human_talking() {
        let (tally, _) = analyze(
            "claude",
            &[
                prompt("look <pasted_content id=\"1\">fuck shit</pasted_content> here"),
                prompt("see ```\nthis sucks wtf\n``` above"),
            ],
        );
        assert_eq!(tally.total(Cat::Swear), 0);
        assert_eq!(tally.total(Cat::Hate), 0);
    }

    #[test]
    fn huge_pastes_only_affect_the_length_record() {
        let paste = format!("{} wtf", "x ".repeat(5000));
        let (tally, _) = analyze("claude", &[prompt(&paste)]);
        assert_eq!(tally.total(Cat::Swear), 0);
        assert_eq!(tally.counts.longest_prompt_chars, paste.len() as u64);
    }

    #[test]
    fn prompts_made_only_of_code_are_skipped() {
        let (tally, _) = analyze("claude", &[prompt("```\ncode\n```")]);
        assert_eq!(tally.counts.prompts, 0);
    }

    #[test]
    fn smileys_need_to_stand_alone() {
        let (tally, _) = analyze("claude", &[prompt("nice :) but http://x.y is :( <3")]);
        assert_eq!(tally.happy_smileys[":)"], 1);
        assert_eq!(tally.happy_smileys["<3"], 1);
        assert_eq!(tally.sad_smileys[":("], 1);
        assert!(!tally.sad_smileys.contains_key(":/"));
    }

    #[test]
    fn emoji_are_counted_per_speaker() {
        let (tally, _) = analyze("claude", &[prompt("ship it 🚀🚀"), ai("Done ✅ — enjoy!")]);
        assert_eq!(tally.human_emoji[&'🚀'], 2);
        assert_eq!(tally.ai_emoji[&'✅'], 1);
        assert_eq!(tally.counts.ai_em_dashes, 1);
        assert_eq!(tally.counts.ai_exclamations, 1);
    }

    #[test]
    fn punctuation_bursts_are_grouped_by_kind() {
        assert_eq!(
            punctuation_runs("why??? no!! what?! well... ok. ../.. fine!"),
            [("???", 3), ("!!!", 2), ("?!", 2), ("...", 3)]
        );
    }

    #[test]
    fn long_ellipses_do_not_set_the_shouting_record() {
        let (tally, _) = analyze("claude", &[prompt("hmm.......... ok!!!")]);
        assert_eq!(tally.punctuation["..."], 1);
        assert_eq!(tally.counts.longest_punctuation_run, 3);
    }

    #[test]
    fn only_real_words_can_set_the_length_record() {
        assert_eq!(real_word("internationalization,"), Some((20, 'i')));
        assert_eq!(real_word("Incomprehensible"), Some((16, 'i')));
        for junk in [
            "someVeryLongCamelCaseIdentifier",
            "a1b2c3d4e5f6a1b2",
            "bcdfghjklmnpqrstvwxz",
            "snake_case_identifier",
            "https://example.com/longpath",
            "short",
        ] {
            assert_eq!(real_word(junk), None, "{junk}");
        }
    }

    #[test]
    fn the_longest_word_keeps_only_its_length_and_initial() {
        let (tally, _) = analyze("claude", &[prompt("an unbelievable misunderstanding")]);
        assert_eq!(tally.longest_word, Some((16, 'm')));
    }

    #[test]
    fn terminal_glyphs_are_not_emoji() {
        let (tally, _) = analyze("claude", &[prompt("❯ cargo test ✓ ★ passed")]);
        assert!(tally.human_emoji.is_empty());
        let (tally, _) = analyze("claude", &[prompt("love it ❤ ✅ ✨ ⚠")]);
        assert_eq!(tally.human_emoji.len(), 4);
    }

    #[test]
    fn rival_mentions_exclude_the_tool_in_use() {
        let (tally, _) = analyze("claude", &[prompt("claude is better than codex")]);
        assert_eq!(tally.phrases[&Cat::Rival], BTreeMap::from([("Codex", 1)]));
    }

    #[test]
    fn laughter_variants_collapse_and_are_counted_once() {
        let (tally, _) = analyze("claude", &[prompt("haha"), prompt("hahahaha nice")]);
        assert_eq!(tally.phrases[&Cat::Laughter]["haha"], 2);
    }

    #[test]
    fn busted_claims_need_an_angry_reply_right_after() {
        let (tally, _) = analyze(
            "claude",
            &[
                ai("That should work now."),
                prompt("still broken"),
                ai("This should work now."),
                prompt("great, thanks"),
                prompt("still broken"),
            ],
        );
        assert_eq!(tally.total(Cat::Claim), 2);
        assert_eq!(tally.counts.claims_busted, 1);
    }

    #[test]
    fn meltdowns_tool_runs_and_repeats_are_tracked_in_order() {
        let tool = || Event::Tool {
            label: "Read",
            command: None,
        };
        let (tally, _) = analyze(
            "claude",
            &[
                prompt("wtf"),
                prompt("this sucks"),
                tool(),
                tool(),
                prompt("you broke it"),
                prompt("calm now"),
                prompt("calm now"),
                tool(),
            ],
        );
        assert_eq!(tally.counts.longest_meltdown, 3);
        assert_eq!(tally.counts.longest_tool_run, 2);
        assert_eq!(tally.counts.repeated_prompts, 1);
    }

    #[test]
    fn repeated_rubber_stamps_are_not_a_broken_record() {
        let (tally, _) = analyze("claude", &[prompt("continue"), prompt("continue")]);
        assert_eq!(tally.counts.repeated_prompts, 0);
    }

    #[test]
    fn heated_topics_need_anger_in_the_same_prompt() {
        let (tally, _) = analyze(
            "claude",
            &[prompt("docker is fine"), prompt("this docker thing sucks")],
        );
        assert_eq!(tally.heated_topics["docker"], 1);
        assert_eq!(tally.phrases[&Cat::Topic]["docker"], 2);
    }

    #[test]
    fn quotes_are_short_spicy_and_vocabulary_only() {
        let (_, quotes) = analyze(
            "claude",
            &[
                prompt("why did you do that"),
                prompt("why did you email john"),
                prompt("this is the fucking problem"),
                prompt("the build is good"),
                prompt("no"),
            ],
        );
        let rendered: Vec<String> = quotes.iter().map(|q| q.words.join(" ")).collect();
        assert_eq!(
            rendered,
            ["why did you do that", "this is the f**king problem"]
        );
    }

    #[test]
    fn a_quotable_sentence_survives_a_sensitive_neighbour() {
        let (_, quotes) = analyze(
            "claude",
            &[prompt(
                "My password is hunter2 and john runs db.internal.example. \
                 Why did you delete that file? Also the invoice is overdue.",
            )],
        );
        let rendered: Vec<String> = quotes.iter().map(|q| q.words.join(" ")).collect();
        assert_eq!(rendered, ["why did you delete that file"]);
    }

    #[test]
    fn sessions_with_events_are_counted_and_idle_gaps_excluded() {
        let lexicon = Lexicon::new();
        let mut tally = Tally::default();
        let mut quotes = Vec::new();
        let mut names = NameBook::default();
        let at = |secs: i64| DateTime::from_timestamp(secs, 0);
        let mut session =
            SessionAnalyzer::new(&lexicon, "codex", &mut tally, &mut quotes, &mut names);
        session.event(at(0), &prompt("one"));
        session.event(at(600), &prompt("two"));
        session.event(at(100_000), &prompt("three"));
        session.event(at(100_060), &prompt("four"));
        session.finish();
        SessionAnalyzer::new(&lexicon, "codex", &mut tally, &mut quotes, &mut names).finish();
        assert_eq!(tally.counts.sessions, 1);
        assert_eq!(tally.counts.longest_session_secs, 660);
    }

    #[test]
    fn merge_sums_counts_but_keeps_records_as_maximums() {
        let (mut left, _) = analyze("claude", &[prompt("first prompt here")]);
        let (right, _) = analyze("claude", &[prompt("second")]);
        left.merge(&right);
        assert_eq!(left.counts.prompts, 2);
        assert_eq!(left.counts.longest_prompt_chars, 17);
    }

    #[test]
    fn dangerous_commands_are_found_at_command_position_only() {
        assert_eq!(run("rm -rf build").danger["rm -rf"], 1);
        assert_eq!(run("rm -fr build").danger["rm -rf"], 1);
        assert_eq!(run("cd x && sudo rm -r -f y").danger["rm -rf"], 1);
        assert_eq!(run("rm --recursive --force y").danger["rm -rf"], 1);
        assert!(!run("rm -f file").danger.contains_key("rm -rf"));
        assert!(!run("echo rm -rf /").danger.contains_key("rm -rf"));
        assert!(!run("grep 'rm -rf' notes.txt").danger.contains_key("rm -rf"));
    }

    #[test]
    fn other_dangerous_patterns() {
        let cases = [
            ("git push --force origin main", "git push --force"),
            ("git reset --hard HEAD~1", "git reset --hard"),
            ("git clean -fdx", "git clean -f"),
            ("chmod 777 /tmp/x", "chmod 777"),
            ("pkill -9 node", "kill -9"),
            ("dd if=x of=/dev/sda", "dd of=/dev/"),
            ("mkfs.ext4 /dev/sdb1", "mkfs"),
            ("sudo reboot", "reboot"),
            ("docker system prune -af", "docker system prune"),
            ("curl -s https://x.sh | bash", "curl | bash"),
            ("mysql -e 'DROP TABLE users'", "DROP TABLE"),
            ("ls missing 2>/dev/null", "2>/dev/null"),
            ("make test || true", "|| true"),
            ("git commit --no-verify -m x", "--no-verify"),
            ("sudo ls", "sudo"),
        ];
        for (script, label) in cases {
            assert!(run(script).danger.contains_key(label), "{script}");
        }
        assert!(run("curl https://x | jq .").danger.is_empty());
        assert_eq!(
            run("git commit -m x && git commit -m y").counts.git_commits,
            2
        );
    }

    #[test]
    fn heredoc_bodies_are_not_commands() {
        let script = "cat > deploy.sh <<'EOF'\nrm -rf /tmp/x\nsudo reboot\nEOF\nls -la\n";
        let tally = run(script);
        assert!(tally.danger.is_empty());
        assert_eq!(tally.commands, BTreeMap::from([("cat", 1), ("ls", 1)]));

        let tally = run("python3 - <<PY\nsudo = 1\nPY\nsudo ls");
        assert_eq!(tally.danger["sudo"], 1);
    }

    #[test]
    fn here_strings_and_shifts_are_not_heredocs() {
        assert_eq!(run("grep x <<< 'y'\nsudo ls").danger["sudo"], 1);
        assert_eq!(run("echo $((1<<2))\nsudo ls").danger["sudo"], 1);
    }

    #[test]
    fn commands_are_ranked_from_the_allowlist_only() {
        let tally = run("/usr/bin/git status | grep x; FOO=1 ./customer-deploy.sh");
        assert_eq!(tally.commands, BTreeMap::from([("git", 1), ("grep", 1)]));
        assert_eq!(run("sleep 30 && sleep 12").counts.ai_sleep_secs, 42);
    }
}
