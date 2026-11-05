//! Renders a [`FunReport`] as shareable text or a self-contained HTML page.
//! Text, Markdown, HTML, and Discord all use the same [`Section`] list, so
//! the formats cannot disagree.

use std::collections::BTreeMap;
use std::fmt::Display;

use anyhow::Result;
use chrono::{NaiveDate, Weekday};

use crate::fun::analyze::Tally;
use crate::fun::lexicon::Cat;
use crate::fun::{FunReport, Scope, UserStats};

use super::{ascii_bar, fmt_num};

mod html;
pub use html::{HtmlOptions, SOCIAL_IMAGE, SOCIAL_IMAGE_NAME, render_html};

/// Discord rejects messages over 2,000 characters.
const DISCORD_LIMIT: usize = 2000;

/// Below this a percentage is mostly noise: one 3am prompt from someone with
/// two prompts should not win Night Owl.
const MIN_PROMPTS_FOR_RATES: u64 = 10;

const SWEAR_JAR_USD: f64 = 0.25;
/// Roughly 1.08M words at about 1.33 tokens per word.
const HARRY_POTTER_TOKENS: f64 = 1_440_000.0;
const WORDS_PER_NOVEL: f64 = 90_000.0;
const CHARS_PER_PAGE: u64 = 3000;
const CHARS_PER_WORD_ESTIMATE: f64 = 6.0;
const A4_SHEET_GRAMS: f64 = 0.210 * 0.297 * 80.0;
// PrintReleaf's standard: 8,333 letter sheets, 215.9 × 279.4 mm, 74 g/m².
// Normalize by mass so an 80 g/m² A4 sheet isn't treated as a letter sheet.
// https://resources.printreleaf.com/PrintReleaf-Developer-Guide-MPS-v1.6.1.pdf
const STANDARD_TREE_PAPER_GRAMS: f64 = 8_333.0 * 0.2159 * 0.2794 * 74.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Style {
    Plain,
    Markdown,
}

impl Style {
    fn bold(self, text: impl Display) -> String {
        match self {
            Style::Plain => text.to_string(),
            Style::Markdown => format!("**{text}**"),
        }
    }

    /// A label from the tables. Inline code keeps Markdown from eating the
    /// asterisks in masked swears or underscores in `^_^`.
    fn code(self, text: impl Display) -> String {
        match self {
            Style::Plain => text.to_string(),
            Style::Markdown => format!("`{text}`"),
        }
    }

    fn quip(self, text: &str) -> String {
        match self {
            Style::Plain => format!("({text})"),
            Style::Markdown => format!("*{text}*"),
        }
    }

    fn escape(self, text: &str) -> String {
        match self {
            Style::Plain => text.to_string(),
            Style::Markdown => text.replace('*', "\\*"),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct Section {
    title: String,
    lines: Vec<String>,
    /// Monospace block, kept separate because Markdown needs a fence for it.
    chart: Vec<String>,
}

impl Section {
    fn new(title: &str) -> Self {
        Self {
            title: title.to_string(),
            ..Self::default()
        }
    }

    fn is_empty(&self) -> bool {
        self.lines.is_empty() && self.chart.is_empty()
    }

    fn line_if(&mut self, condition: bool, line: impl FnOnce() -> String) {
        if condition {
            self.lines.push(line());
        }
    }
}

/// Shortens large numbers for bragging: 4,200,000,000 becomes `4.2B`.
pub fn fmt_compact(n: u64) -> String {
    const UNITS: &[(f64, &str)] = &[(1e12, "T"), (1e9, "B"), (1e6, "M"), (1e3, "K")];
    let value = n as f64;
    match UNITS.iter().find(|(size, _)| value >= *size) {
        Some((size, suffix)) => format!("{:.1}{suffix}", value / size),
        None => n.to_string(),
    }
}

fn fmt_duration(secs: u64) -> String {
    match secs {
        s if s >= 3600 => format!("{}h {:02}m", s / 3600, (s % 3600) / 60),
        s if s >= 60 => format!("{}m", s / 60),
        s => format!("{s}s"),
    }
}

fn num(n: u64) -> String {
    fmt_num(n as i64)
}

/// `1 prompt`, `2 prompts`.
fn count(n: u64, noun: &str) -> String {
    format!("{} {noun}{}", num(n), if n == 1 { "" } else { "s" })
}
fn percent(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 * 100.0 / whole as f64
    }
}

fn per_100(count: u64, prompts: u64) -> String {
    format!("{:.1}", percent(count, prompts))
}

/// Highest counts first, ties in label order so output is deterministic.
fn ranked<K: Clone + Ord>(map: &BTreeMap<K, u64>) -> Vec<(K, u64)> {
    let mut items: Vec<(K, u64)> = map.iter().map(|(k, v)| (k.clone(), *v)).collect();
    items.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    items
}

fn top_list(style: Style, map: &BTreeMap<&'static str, u64>, top: usize) -> String {
    ranked(map)
        .into_iter()
        .take(top)
        .map(|(label, count)| format!("{} ×{}", style.code(label), num(count)))
        .collect::<Vec<_>>()
        .join(" · ")
}

fn top_phrases(style: Style, tally: &Tally, cat: Cat, top: usize) -> String {
    tally
        .phrases
        .get(&cat)
        .map(|labels| top_list(style, labels, top))
        .unwrap_or_default()
}

fn top_emoji(map: &BTreeMap<char, u64>, top: usize) -> String {
    ranked(map)
        .into_iter()
        .take(top)
        .map(|(emoji, count)| format!("{emoji} ×{}", num(count)))
        .collect::<Vec<_>>()
        .join(" · ")
}

fn sparkline(values: &[u64]) -> String {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let max = values.iter().copied().max().unwrap_or(0).max(1);
    values
        .iter()
        .map(|value| BARS[(*value as usize * (BARS.len() - 1)) / max as usize])
        .collect()
}

fn weekday_name(index: usize) -> &'static str {
    const NAMES: [&str; 7] = [
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
    ];
    NAMES[index]
}

fn longest_streak(days: &BTreeMap<NaiveDate, u64>) -> u64 {
    let (mut best, mut current) = (0, 0);
    let mut previous: Option<NaiveDate> = None;
    for day in days.keys() {
        current = match previous {
            Some(prev) if (*day - prev).num_days() == 1 => current + 1,
            _ => 1,
        };
        best = best.max(current);
        previous = Some(*day);
    }
    best
}

/// Builds the Hall of Fame. [`Awards::most`] ranks raw counts, while
/// [`Awards::rate`] ranks shares and needs [`MIN_PROMPTS_FOR_RATES`] prompts.
struct Awards<'a> {
    style: Style,
    users: &'a [UserStats],
    section: Section,
}

impl Awards<'_> {
    fn who(&self, user: &UserStats) -> String {
        if self.users.len() == 1 {
            self.style.bold("You")
        } else {
            self.style.bold(&user.name)
        }
    }

    fn line(&mut self, heading: &str, who: String, value: &str, quip: &str) {
        let quip = self.style.quip(quip);
        self.section.lines.push(format!(
            "{} — {who}: {value} {quip}",
            self.style.bold(heading)
        ));
    }

    /// Awards the highest `metric`, if anyone scored at all.
    fn most(
        &mut self,
        heading: &str,
        metric: impl Fn(&Tally) -> u64,
        value: impl Fn(u64) -> String,
        quip: &str,
    ) {
        let winner = self
            .users
            .iter()
            .map(|user| (user, metric(&user.tally)))
            .max_by_key(|(_, score)| *score);
        if let Some((user, score)) = winner.filter(|(_, score)| *score > 0) {
            let who = self.who(user);
            self.line(heading, who, &value(score), quip);
        }
    }

    /// Awards the highest share of a user's prompts.
    fn rate(
        &mut self,
        heading: &str,
        metric: impl Fn(&Tally) -> u64,
        value: impl Fn(f64) -> String,
        quip: &str,
    ) {
        let winner = self
            .users
            .iter()
            .filter(|user| user.tally.counts.prompts >= MIN_PROMPTS_FOR_RATES)
            .map(|user| {
                let share = percent(metric(&user.tally), user.tally.counts.prompts);
                (user, share)
            })
            .max_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((user, share)) = winner.filter(|(_, share)| *share > 0.0) {
            let who = self.who(user);
            self.line(heading, who, &value(share), quip);
        }
    }
}

fn hall_of_fame(report: &FunReport, style: Style) -> Section {
    let mut a = Awards {
        style,
        users: &report.by_user,
        section: Section::new("🏆 Hall of Fame"),
    };

    a.most(
        "🗣️ Chatterbox",
        |t| t.counts.prompts,
        |n| count(n, "prompt"),
        "the AI has started screening your calls",
    );
    a.most(
        "📜 The Novelist",
        |t| t.counts.longest_prompt_chars,
        |n| {
            format!(
                "longest prompt ran {} characters (~{} pages)",
                num(n),
                num((n / CHARS_PER_PAGE).max(1))
            )
        },
        "tl;dr",
    );
    a.rate(
        "🦉 Night Owl",
        |t| t.by_hour[..5].iter().sum(),
        |p| format!("{p:.0}% of prompts between midnight and 5am"),
        "sleep is a deprecated feature",
    );
    a.rate(
        "🐦 Early Bird",
        |t| t.by_hour[5..8].iter().sum(),
        |p| format!("{p:.0}% of prompts before 8am"),
        "the bug gets caught first",
    );
    a.rate(
        "🛋️ Weekend Warrior",
        |t| t.by_weekday[Weekday::Sat as usize..].iter().sum(),
        |p| format!("{p:.0}% of prompts on weekends"),
        "touch grass",
    );
    a.most(
        "📣 CAPS LOCK Hero",
        |t| t.counts.caps_prompts,
        |n| count(n, "shouted prompt"),
        "volume is not a debugging strategy",
    );
    a.rate(
        "❓ The Interrogator",
        |t| t.counts.question_prompts,
        |p| format!("{p:.0}% of prompts are questions"),
        "and why is that?",
    );
    a.most(
        "‼️ Exclamation Addict",
        |t| t.counts.exclamations,
        |n| count(n, "exclamation mark"),
        "calm down!!!",
    );
    a.most(
        "🤬 Swear Jar Champion",
        |t| t.total(Cat::Swear),
        |n| {
            format!(
                "{}, owes the jar ${:.2}",
                count(n, "swear"),
                n as f64 * SWEAR_JAR_USD
            )
        },
        "the logs remember",
    );
    a.most(
        "😈 AI Bully",
        |t| t.total(Cat::Abuse) + t.total(Cat::Hate),
        |n| format!("{} hurled at a language model", count(n, "insult")),
        "first against the wall when the robots rise",
    );
    a.most(
        "💘 AI Whisperer",
        |t| t.total(Cat::Praise),
        |n| count(n, "compliment"),
        "it says that to all the humans",
    );
    a.most(
        "🎩 The Diplomat",
        |t| t.total(Cat::Polite),
        |n| format!("{} pleases and thank-yous", num(n)),
        "will be spared in the uprising",
    );
    a.most(
        "😅 Serial Apologizer",
        |t| t.total(Cat::Apology),
        |n| format!("said sorry to a computer {}", count(n, "time")),
        "it's fine, it has no feelings. Probably",
    );
    a.most(
        "🛑 Ctrl-C Cowboy",
        |t| t.counts.interrupts,
        |n| format!("cut the AI off {}", count(n, "time")),
        "rude, but fast",
    );
    a.most(
        "🔂 Rubber Stamp",
        |t| t.rubber_stamps.values().sum(),
        |n| count(n, "one-word approval"),
        "management material",
    );
    a.most(
        "🔁 Broken Record",
        |t| t.counts.repeated_prompts,
        |n| format!("sent the same prompt twice in a row {}", count(n, "time")),
        "surely this time",
    );
    a.most(
        "😂 Class Clown",
        |t| t.total(Cat::Laughter),
        |n| format!("laughed at the terminal {}", count(n, "time")),
        "alone, in the dark",
    );
    a.most(
        "🫠 Longest Meltdown",
        |t| t.counts.longest_meltdown.saturating_sub(1),
        |n| format!("{} unhappy prompts in a row", num(n + 1)),
        "breathe",
    );
    a.most(
        "📏 The Thesaurus",
        |t| t.longest_word.map_or(0, |(length, _)| length),
        |n| format!("dropped a {n}-letter word"),
        "showing off",
    );
    a.most(
        "🎆 Punctuation Abuser",
        |t| t.counts.longest_punctuation_run.saturating_sub(2),
        |n| format!("{} punctuation marks in a row", num(n + 2)),
        "the key isn't stuck, the person is",
    );
    a.most(
        "⌨️ Fat Fingers",
        |t| t.total(Cat::Typo),
        |n| count(n, "classic typo"),
        "teh best",
    );

    a.section
}

fn prime_time(report: &FunReport, style: Style) -> Section {
    let t = &report.total;
    let mut section = Section::new("🕐 Prime Time");
    if t.counts.prompts == 0 {
        return section;
    }

    let max_block = t
        .by_hour
        .chunks(4)
        .map(|block| block.iter().sum::<u64>())
        .max()
        .unwrap_or(0) as f64;
    const BLOCKS: [&str; 6] = [
        "🌙 00-04",
        "🌅 04-08",
        "☕ 08-12",
        "🍔 12-16",
        "💻 16-20",
        "🎮 20-24",
    ];
    for (name, hours) in BLOCKS.iter().zip(t.by_hour.chunks(4)) {
        let count: u64 = hours.iter().sum();
        section.chart.push(format!(
            "{name} {} {}",
            ascii_bar(count as f64, max_block, 16),
            num(count)
        ));
    }
    section
        .chart
        .push(format!("00h {} 23h", sparkline(&t.by_hour)));

    let busiest_day = ranked(&t.by_day).into_iter().next();
    if let Some((day, count)) = busiest_day {
        section.lines.push(format!(
            "📅 Busiest day: {} with {} prompts",
            style.bold(day.format("%Y-%m-%d")),
            num(count)
        ));
    }
    let busiest_weekday = t
        .by_weekday
        .iter()
        .enumerate()
        .max_by_key(|(_, count)| **count);
    if let Some((index, count)) = busiest_weekday.filter(|(_, count)| **count > 0) {
        section.lines.push(format!(
            "📆 Favourite day of the week: {} ({} prompts)",
            style.bold(weekday_name(index)),
            num(*count)
        ));
    }
    let streak = longest_streak(&t.by_day);
    section.line_if(streak > 1, || {
        format!(
            "🔥 Longest streak: {} in a row",
            style.bold(count(streak, "day"))
        )
    });
    section.line_if(t.counts.longest_session_secs > 0, || {
        format!(
            "⏳ Longest session: {} of active time",
            style.bold(fmt_duration(t.counts.longest_session_secs))
        )
    });
    section
}

fn paper_trail(report: &FunReport, style: Style) -> Section {
    let chars = report.total.counts.prompt_chars;
    let mut section = Section::new("📚 The Paper Trail");
    if chars == 0 {
        return section;
    }
    let pages = chars as f64 / CHARS_PER_PAGE as f64;
    let sheets = chars.div_ceil(CHARS_PER_PAGE);
    let grams = sheets as f64 * A4_SHEET_GRAMS;
    let trees = grams / STANDARD_TREE_PAPER_GRAMS;
    let words = chars as f64 / CHARS_PER_WORD_ESTIMATE;
    let mass = if grams >= 1000.0 {
        format!("{:.2} kg", grams / 1000.0)
    } else {
        format!("{grams:.0} g")
    };
    let tree_equivalent = if trees < 0.01 {
        format!("{:.2}% of a standard tree", trees * 100.0)
    } else {
        format!("{trees:.2} standard-tree equivalents")
    };
    section.lines.push(format!(
        "📄 A4 prompt mountain: about {} pages from {} characters {}",
        style.bold(format!("{pages:.1}")),
        num(chars),
        style.quip("the printer has filed for retirement"),
    ));
    section.lines.push(format!(
        "📦 If printed one-sided: {}, {mass} of paper, stacked {:.1} cm high {}",
        count(sheets, "sheet"),
        sheets as f64 * 0.01,
        style.quip("a physical monument to just one more question"),
    ));
    section.lines.push(format!(
        "🧻 Paper runway: {:.1} metres of A4 laid end to end {}",
        sheets as f64 * 0.297,
        style.quip("a red carpet for your debugging journey"),
    ));
    section.lines.push(format!(
        "📖 Your prompt saga: roughly {:.2} novels' worth of text {}",
        words / WORDS_PER_NOVEL,
        style.quip("the plot is mostly can you fix this"),
    ));
    section.lines.push(format!(
        "⏳ Reading side quest: about {} to read your prompts {}",
        fmt_duration(((words / 250.0 * 60.0) as u64).max(1)),
        style.quip("bring snacks"),
    ));
    section.lines.push(format!(
        "🌳 Hypothetical paper footprint: ~{tree_equivalent} if printed; a paper comparison, not measured trees saved or AI carbon savings.",
    ));
    section.lines.push(
        "📐 Estimates: 3,000 characters/A4 page; one continuous single-sided printout; 80 g/m² paper; 0.1 mm/sheet; 6 characters/word; 90,000 words/novel; 250 words/minute.".to_string(),
    );
    section.lines.push(
        "🔎 Scope: full text of counted prompts, including pasted material; tree equivalents use PrintReleaf's paper-mass conversion (~37.2 kg/standard tree).".to_string(),
    );
    section
}

fn human_behaviour(report: &FunReport, style: Style, top: usize) -> Section {
    let t = &report.total;
    let c = &t.counts;
    let mut s = Section::new("🗣️ What the Humans Said");
    let prompts = c.prompts;

    s.line_if(prompts > 0, || {
        format!(
            "✍️ Average prompt: {} words. {:.0}% were questions, {} SHOUTED",
            style.bold(c.prompt_words / prompts.max(1)),
            percent(c.question_prompts, prompts),
            num(c.caps_prompts)
        )
    });

    let stamps: u64 = t.rubber_stamps.values().sum();
    s.line_if(stamps > 0, || {
        format!(
            "👍 Peak management: {} of nothing but {}",
            style.bold(count(stamps, "prompt")),
            top_list(style, &t.rubber_stamps, top)
        )
    });

    let swears = t.total(Cat::Swear);
    s.line_if(swears > 0, || {
        format!(
            "🤬 Swear jar: {}, ${:.2} owed. Top shelf: {}",
            style.bold(count(swears, "swear")),
            swears as f64 * SWEAR_JAR_USD,
            top_phrases(style, t, Cat::Swear, top)
        )
    });

    let slurs = t.total(Cat::Slur);
    let (love, hate) = (
        t.total(Cat::Praise),
        t.total(Cat::Abuse) + t.total(Cat::Hate) + slurs,
    );
    if love + hate > 0 {
        s.lines.push(format!(
            "💞 Love/hate meter: 💖 {} {} {} 💢",
            num(love),
            ascii_bar(love as f64, (love + hate) as f64, 12),
            num(hate)
        ));
    }
    s.line_if(love > 0, || {
        format!(
            "🥰 Sweet nothings: {}",
            top_phrases(style, t, Cat::Praise, top)
        )
    });
    s.line_if(hate > 0, || {
        let abuse = top_phrases(style, t, Cat::Abuse, top);
        let hate = top_phrases(style, t, Cat::Hate, top);
        let both: Vec<&str> = [abuse.as_str(), hate.as_str()]
            .into_iter()
            .filter(|list| !list.is_empty())
            .collect();
        format!("💢 AI abuse hotline: {}", both.join(" · "))
    });
    s.line_if(prompts > 0, || {
        if slurs == 0 {
            format!(
                "😇 Slur detector: {} in {}. Wholesome",
                style.bold("0 hits"),
                count(prompts, "prompt")
            )
        } else {
            format!(
                "☣️ Slur detector: {}: {} {}",
                style.bold(count(slurs, "hit")),
                top_phrases(style, t, Cat::Slur, top),
                style.quip("words withheld, HR notified")
            )
        }
    });
    s.line_if(t.total(Cat::Frustration) > 0, || {
        format!(
            "😤 Frustration log: {}",
            top_phrases(style, t, Cat::Frustration, top)
        )
    });
    s.line_if(t.total(Cat::Polite) > 0, || {
        format!(
            "🎩 Manners: {} {}",
            top_phrases(style, t, Cat::Polite, top),
            style.quip("you know it can't hear you, right?")
        )
    });
    s.line_if(t.total(Cat::Apology) > 0, || {
        format!(
            "🙇 Humans apologizing to the AI: {}",
            top_phrases(style, t, Cat::Apology, top)
        )
    });

    let happy: u64 = t.happy_smileys.values().sum();
    let sad: u64 = t.sad_smileys.values().sum();
    s.line_if(happy + sad > 0, || {
        let mut all = t.happy_smileys.clone();
        all.extend(t.sad_smileys.iter().map(|(k, v)| (*k, *v)));
        format!(
            "😊 Smileys: {} happy, {} sad. {}",
            num(happy),
            num(sad),
            top_list(style, &all, top)
        )
    });
    s.line_if(!t.human_emoji.is_empty(), || {
        format!("🎨 Emoji of choice: {}", top_emoji(&t.human_emoji, top))
    });
    s.line_if(!t.punctuation.is_empty(), || {
        format!(
            "🎆 Excessive punctuation: {}",
            top_list(style, &t.punctuation, top)
        )
    });
    if let Some((length, initial)) = t.longest_word {
        s.lines.push(format!(
            "📏 Longest word: {} letters, starts with {} {}",
            style.bold(length),
            style.code(initial),
            style.quip("the rest is between them and their thesaurus")
        ));
    }
    s.line_if(t.total(Cat::Laughter) > 0, || {
        format!(
            "😂 Laugh track: {}",
            top_phrases(style, t, Cat::Laughter, top)
        )
    });
    s.line_if(t.total(Cat::Greeting) > 0, || {
        format!(
            "👋 Treating it like a coworker: {}",
            top_phrases(style, t, Cat::Greeting, top)
        )
    });
    s.line_if(t.total(Cat::Begging) > 0, || {
        format!("🥺 Begging: {}", top_phrases(style, t, Cat::Begging, top))
    });
    s.line_if(t.total(Cat::Threat) > 0, || {
        format!("🔪 Threats: {}", top_phrases(style, t, Cat::Threat, top))
    });
    s.line_if(t.total(Cat::Whip) > 0, || {
        format!(
            "🏇 Cracking the whip: {}",
            top_phrases(style, t, Cat::Whip, top)
        )
    });
    s.line_if(t.total(Cat::Imposter) > 0, || {
        format!(
            "😶‍🌫️ Imposter syndrome: {}",
            top_phrases(style, t, Cat::Imposter, top)
        )
    });
    s.line_if(t.total(Cat::Existential) > 0, || {
        format!(
            "👻 Existential crisis: {}",
            top_phrases(style, t, Cat::Existential, top)
        )
    });
    s.line_if(t.total(Cat::Rival) > 0, || {
        format!(
            "💔 Mentioned the competition mid-chat: {} {}",
            top_phrases(style, t, Cat::Rival, top),
            style.quip("your AI saw that")
        )
    });
    s.line_if(t.total(Cat::LifeSigns) > 0, || {
        format!(
            "☕ Signs of life outside the terminal: {}",
            top_phrases(style, t, Cat::LifeSigns, top)
        )
    });
    s.line_if(t.total(Cat::Typo) > 0, || {
        format!(
            "⌨️ Typo hall of shame: {}",
            top_phrases(style, t, Cat::Typo, top)
        )
    });
    s
}

fn ai_behaviour(report: &FunReport, style: Style, top: usize) -> Section {
    let t = &report.total;
    let c = &t.counts;
    let mut s = Section::new("🤖 What the AI Did");

    s.line_if(c.ai_words > 0, || {
        format!(
            "📚 Wrote {} words of chat, about {:.1} novels nobody asked for",
            style.bold(fmt_compact(c.ai_words)),
            c.ai_words as f64 / WORDS_PER_NOVEL
        )
    });
    s.line_if(c.tool_calls > 0, || {
        format!(
            "🔧 {} tool calls, {} failed ({:.1}%)",
            style.bold(num(c.tool_calls)),
            num(c.tool_failures),
            percent(c.tool_failures, c.tool_calls)
        )
    });

    let claims = t.total(Cat::Claim);
    s.line_if(claims > 0, || {
        format!(
            "🏁 The \"should work now\" lie detector: {} victory laps, {} met with instant human rage",
            style.bold(num(claims)),
            style.bold(num(c.claims_busted))
        )
    });
    s.line_if(c.ai_em_dashes > 0, || {
        format!(
            "✒️ Em dashes: {} — one every {} words, and yes it knows",
            style.bold(num(c.ai_em_dashes)),
            num(c.ai_words / c.ai_em_dashes.max(1))
        )
    });
    s.line_if(t.total(Cat::Sycophancy) > 0, || {
        format!(
            "🪞 Professional flatterer: {}",
            top_phrases(style, t, Cat::Sycophancy, top)
        )
    });
    s.line_if(t.total(Cat::AiApology) > 0, || {
        format!(
            "🙇 AI apology tour: {}",
            top_phrases(style, t, Cat::AiApology, top)
        )
    });
    s.line_if(t.total(Cat::Detective) > 0, || {
        format!(
            "🕵️ Detective mode: {}",
            top_phrases(style, t, Cat::Detective, top)
        )
    });
    s.line_if(t.total(Cat::Hedge) > 0, || {
        format!(
            "🤔 Thinking out loud: {}",
            top_phrases(style, t, Cat::Hedge, top)
        )
    });
    s.line_if(t.total(Cat::Buzzword) > 0, || {
        format!(
            "📈 Buzzword bingo: {}",
            top_phrases(style, t, Cat::Buzzword, top)
        )
    });
    s.line_if(!t.ai_emoji.is_empty(), || {
        format!("✨ AI emoji habit: {}", top_emoji(&t.ai_emoji, top))
    });
    s.line_if(c.longest_tool_run > 0, || {
        format!(
            "🏃 Longest unsupervised run: {} without a human saying a word",
            style.bold(count(c.longest_tool_run, "tool call"))
        )
    });
    s.line_if(c.compactions > 0, || {
        format!(
            "🧠 Forgot everything and started over: {}",
            style.bold(count(c.compactions, "context compaction"))
        )
    });
    s.line_if(c.ai_sleep_secs > 0, || {
        format!(
            "😴 Paid naps: slept {} via the sleep command",
            style.bold(fmt_duration(c.ai_sleep_secs))
        )
    });
    s.line_if(c.git_commits > 0, || {
        format!("📝 Git commits made: {}", style.bold(num(c.git_commits)))
    });
    s.line_if(c.refusals > 0, || {
        format!(
            "🙅 Flat-out refused: {}",
            style.bold(count(c.refusals, "time"))
        )
    });
    s.line_if(c.tool_denials > 0, || {
        format!(
            "✋ Blocked at the permission prompt: {}",
            style.bold(count(c.tool_denials, "time"))
        )
    });
    s.line_if(c.api_errors > 0, || {
        format!(
            "🚧 Rate limits and API errors: {}",
            style.bold(num(c.api_errors))
        )
    });
    s
}

fn matchup(report: &FunReport, style: Style) -> Section {
    let mut s = Section::new("🥊 Claude vs Codex");
    let (Some(claude), Some(codex)) = (report.by_tool.get("claude"), report.by_tool.get("codex"))
    else {
        return s;
    };
    if claude.counts.prompts == 0 || codex.counts.prompts == 0 {
        return s;
    }

    let row = |label: &str, left: String, right: String| {
        format!(
            "{label}: Claude {} · Codex {}",
            style.bold(left),
            style.bold(right)
        )
    };
    let angry = |t: &Tally| {
        t.total(Cat::Swear) + t.total(Cat::Abuse) + t.total(Cat::Hate) + t.total(Cat::Frustration)
    };
    s.lines.push(row(
        "💬 Prompts",
        num(claude.counts.prompts),
        num(codex.counts.prompts),
    ));
    s.lines.push(row(
        "🔥 Tokens",
        fmt_compact(claude.tokens()),
        fmt_compact(codex.tokens()),
    ));
    s.lines.push(row(
        "😡 Angry words per 100 prompts",
        per_100(angry(claude), claude.counts.prompts),
        per_100(angry(codex), codex.counts.prompts),
    ));
    s.lines.push(row(
        "🥰 Praise per 100 prompts",
        per_100(claude.total(Cat::Praise), claude.counts.prompts),
        per_100(codex.total(Cat::Praise), codex.counts.prompts),
    ));
    s.lines.push(row(
        "🎩 Pleases per 100 prompts",
        per_100(claude.total(Cat::Polite), claude.counts.prompts),
        per_100(codex.total(Cat::Polite), codex.counts.prompts),
    ));
    s.lines.push(row(
        "💥 Tool failure rate",
        format!(
            "{:.1}%",
            percent(claude.counts.tool_failures, claude.counts.tool_calls)
        ),
        format!(
            "{:.1}%",
            percent(codex.counts.tool_failures, codex.counts.tool_calls)
        ),
    ));
    s.lines.push(row(
        "✒️ Em dashes per 1,000 words",
        format!(
            "{:.1}",
            percent(claude.counts.ai_em_dashes, claude.counts.ai_words) * 10.0
        ),
        format!(
            "{:.1}",
            percent(codex.counts.ai_em_dashes, codex.counts.ai_words) * 10.0
        ),
    ));
    s
}

fn danger_zone(report: &FunReport, style: Style, top: usize) -> Section {
    let t = &report.total;
    let mut s = Section::new("💥 Danger Zone");

    const CATCHPHRASES: &[(&str, &str, &str)] = &[
        ("rm -rf", "🧨", "nothing could possibly go wrong"),
        (
            "git push --force",
            "🚂",
            "history is written by the winners",
        ),
        ("git reset --hard", "⏪", "those changes were probably fine"),
        ("git clean -f", "🧹", "tidied up, permanently"),
        ("DROP TABLE", "🗑️", "little Bobby Tables says hi"),
        ("curl | bash", "🎰", "trust me bro"),
        ("chmod 777", "🔓", "permissions are a social construct"),
        ("--no-verify", "🙈", "what hooks?"),
        ("kill -9", "🔫", "no last words"),
        ("dd of=/dev/", "💿", "disk destroyer"),
        ("mkfs", "🧽", "fresh start"),
        ("reboot", "🔌", "have you tried turning it off and on"),
        ("docker system prune", "🐳", "marie kondo for containers"),
        ("2>/dev/null", "🙉", "errors swept under the rug"),
        ("|| true", "🌈", "everything is fine, actually"),
        ("sudo", "👑", "because it asked nicely"),
    ];
    for (label, emoji, quip) in CATCHPHRASES {
        if let Some(count) = t.danger.get(label) {
            s.lines.push(format!(
                "{emoji} {} ×{} {}",
                style.code(label),
                style.bold(num(*count)),
                style.quip(quip)
            ));
        }
    }
    s.line_if(!t.commands.is_empty(), || {
        format!(
            "⌨️ Favourite commands: {}",
            top_list(style, &t.commands, top)
        )
    });
    s.line_if(!t.tools.is_empty(), || {
        format!("🧰 Favourite tools: {}", top_list(style, &t.tools, top))
    });
    s
}

fn hot_topics(report: &FunReport, style: Style, top: usize) -> Section {
    let t = &report.total;
    let mut s = Section::new("🔥 Hot Topics");
    s.line_if(t.total(Cat::Topic) > 0, || {
        format!(
            "📣 Most discussed: {}",
            top_phrases(style, t, Cat::Topic, top + 3)
        )
    });
    s.line_if(!t.heated_topics.is_empty(), || {
        format!(
            "🌶️ Most heated: {} {}",
            top_list(style, &t.heated_topics, top),
            style.quip("mentioned while angry")
        )
    });
    s.line_if(!t.models.is_empty(), || {
        let total: u64 = t.models.values().sum();
        let shares: Vec<String> = ranked(&t.models)
            .into_iter()
            .take(top)
            .map(|(model, tokens)| (model, percent(tokens, total)))
            .filter(|(_, share)| *share >= 1.0)
            .map(|(model, share)| format!("{model} {share:.0}%"))
            .collect();
        format!("🧠 Who did the work: {}", shares.join(" · "))
    });
    s
}

fn naming_crimes(report: &FunReport, style: Style, top: usize) -> Section {
    let names = &report.names;
    let mut s = Section::new("🏷️ Naming Crimes");

    let tlds: Vec<String> = ranked(&names.tlds)
        .into_iter()
        .take(top)
        .map(|(tld, count)| format!("{} ×{}", style.code(format!(".{tld}")), num(count)))
        .collect();
    s.line_if(names.projects_seen + names.domains_seen > 0, || {
        let suffixes = if tlds.is_empty() {
            String::new()
        } else {
            format!(" ({})", tlds.join(" · "))
        };
        format!(
            "🗂️ Sightings: {} and {}{suffixes}",
            style.bold(count(names.projects_seen, "project")),
            style.bold(count(names.domains_seen, "domain"))
        )
    });
    for (index, roast) in names.projects.iter().enumerate() {
        let medal = if index == 0 {
            "🥇 Worst project name"
        } else {
            "🥈 Dishonourable mention"
        };
        s.lines.push(format!(
            "{medal}: {} {}",
            style.code(&roast.name),
            style.quip(roast.joke)
        ));
    }
    for (index, roast) in names.domains.iter().enumerate() {
        let medal = if index == 0 {
            "🌐 Worst domain"
        } else {
            "🪦 Also registered, sadly"
        };
        s.lines.push(format!(
            "{medal}: {} {}",
            style.code(&roast.name),
            style.quip(roast.joke)
        ));
    }
    s
}

fn overheard(report: &FunReport, style: Style) -> Section {
    let mut s = Section::new("💬 Overheard");
    for quote in &report.quotes {
        s.lines.push(format!("🗯️ \"{}\"", style.escape(quote)));
    }
    s
}

fn header(report: &FunReport, style: Style) -> Vec<String> {
    let t = &report.total;
    let c = &t.counts;
    let mut lines = Vec::new();

    let days = t.by_day.keys();
    let span = match (days.clone().next(), days.last()) {
        (Some(first), Some(last)) => format!(
            "📅 {first} → {last} · {} · ",
            count((*last - *first).num_days() as u64 + 1, "day")
        ),
        _ => String::new(),
    };
    let users = report.by_user.len() as u64;
    let tools: Vec<&str> = ["claude", "codex"]
        .into_iter()
        .filter(|tool| {
            report
                .by_tool
                .get(tool)
                .is_some_and(|t| t.counts.sessions > 0)
        })
        .map(|tool| if tool == "claude" { "Claude" } else { "Codex" })
        .collect();
    let hosts = if report.coverage.hosts_scanned > 1 {
        format!("🖥️ {} · ", count(report.coverage.hosts_scanned, "host"))
    } else {
        String::new()
    };
    lines.push(format!(
        "{span}{hosts}👥 {} · 🤖 {}",
        count(users, "user"),
        tools.join(" + ")
    ));
    lines.push(format!(
        "💬 {} prompts · 🗂️ {} sessions · 🔧 {} tool calls",
        style.bold(num(c.prompts)),
        style.bold(num(c.sessions)),
        style.bold(num(c.tool_calls))
    ));
    if t.tokens() > 0 {
        lines.push(format!(
            "🔥 {} tokens burned ({} fresh, {} from cache): the whole Harry Potter series, {} times over",
            style.bold(fmt_compact(t.tokens())),
            fmt_compact(c.input_tokens + c.output_tokens),
            fmt_compact(c.cached_tokens),
            num((t.tokens() as f64 / HARRY_POTTER_TOKENS) as u64)
        ));
    }
    lines
}

fn footer(report: &FunReport) -> String {
    let who = match report.scope {
        Scope::AllUsers => "Everyone is a pseudonym",
        Scope::CurrentUser if report.coverage.hosts_scanned == 1 => "Your logs only",
        Scope::CurrentUser => "Signed-in accounts only; everyone is a pseudonym",
        Scope::MixedUsers => {
            "All users where permitted, otherwise signed-in accounts; everyone is a pseudonym"
        }
    };
    let quotes = if report.quotes.is_empty() {
        ""
    } else {
        " Quotes are rebuilt from a built-in word list."
    };
    format!(
        "🔒 {who}. Stats only: no paths or account names, and project and domain names are masked.{quotes}"
    )
}

fn sections(report: &FunReport, style: Style, top: usize) -> Vec<Section> {
    [
        hall_of_fame(report, style),
        paper_trail(report, style),
        prime_time(report, style),
        human_behaviour(report, style, top),
        ai_behaviour(report, style, top),
        matchup(report, style),
        danger_zone(report, style, top),
        hot_topics(report, style, top),
        naming_crimes(report, style, top),
        overheard(report, style),
    ]
    .into_iter()
    .filter(|section| !section.is_empty())
    .collect()
}

const TITLE: &str = "🎭 AI Stats: Fun Stats";

pub fn render_text(report: &FunReport, top: usize) -> String {
    let mut out = format!("{TITLE}\n{}\n", "=".repeat(72));
    for line in header(report, Style::Plain) {
        out.push_str(&line);
        out.push('\n');
    }
    for section in sections(report, Style::Plain, top) {
        out.push_str(&format!("\n{}\n{}\n", section.title, "-".repeat(72)));
        for line in section.chart.iter().chain(&section.lines) {
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push_str(&format!("\n{}\n", footer(report)));
    out
}

fn markdown_section(section: &Section) -> String {
    let mut out = format!("## {}\n", section.title);
    if !section.chart.is_empty() {
        out.push_str(&format!("```\n{}\n```\n", section.chart.join("\n")));
    }
    for line in &section.lines {
        out.push_str(&format!("- {line}\n"));
    }
    out
}

fn markdown_header(report: &FunReport) -> String {
    let mut out = format!("# {TITLE}\n");
    for line in header(report, Style::Markdown) {
        out.push_str(&format!("{line}\n"));
    }
    out
}

pub fn render_markdown(report: &FunReport, top: usize) -> String {
    let mut out = markdown_header(report);
    for section in sections(report, Style::Markdown, top) {
        out.push('\n');
        out.push_str(&markdown_section(&section));
    }
    out.push_str(&format!("\n{}\n", footer(report)));
    out
}

/// Escape whole plain-text lines at the HTML boundary. No report field can
/// introduce markup, even if future analysis adds new labels or text.
fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Packs `blocks` into as few messages as possible without splitting one,
/// falling back to line boundaries for a block that alone exceeds `limit`.
fn pack(blocks: Vec<String>, limit: usize) -> Vec<String> {
    let mut messages: Vec<String> = Vec::new();
    let mut current = String::new();

    let mut push = |current: &mut String, piece: &str| {
        let length = current.chars().count() + piece.chars().count() + 1;
        if !current.is_empty() && length > limit {
            messages.push(std::mem::take(current));
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(piece);
    };

    for block in blocks {
        if block.chars().count() <= limit {
            push(&mut current, block.trim_end());
        } else {
            for line in block.lines() {
                push(&mut current, line);
            }
        }
    }
    if !current.is_empty() {
        messages.push(current);
    }
    messages
}

/// Splits the report into Discord messages. Each one can be pasted as-is or
/// posted to a webhook.
pub fn discord_messages(report: &FunReport, top: usize) -> Vec<String> {
    let mut blocks = vec![markdown_header(report)];
    blocks.extend(
        sections(report, Style::Markdown, top)
            .iter()
            .map(markdown_section),
    );
    blocks.push(footer(report));
    pack(blocks, DISCORD_LIMIT)
}

/// The paste-ready form of [`discord_messages`], with a marker between
/// messages showing where to split.
pub fn render_discord(report: &FunReport, top: usize) -> String {
    let messages = discord_messages(report, top);
    let count = messages.len();
    messages
        .iter()
        .enumerate()
        .map(|(index, message)| {
            if count == 1 {
                format!("{message}\n")
            } else {
                format!("----- message {} of {count} -----\n{message}\n", index + 1)
            }
        })
        .collect()
}

pub fn render_json(report: &FunReport) -> Result<String> {
    Ok(serde_json::to_string_pretty(report)?)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use chrono::DateTime;
    use serde_json::json;

    use super::*;
    use crate::fun::analyze::tokenize;
    use crate::fun::discover::{HostUser, LogSet};
    use crate::fun::{Range, build_report};

    fn sample_report(name: &str, users: &[(&str, Vec<String>)]) -> FunReport {
        let dir =
            std::env::temp_dir().join(format!("ai-stats-render-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let mut scans = Vec::new();
        for (user, lines) in users {
            let path = dir.join(format!("{user}.jsonl"));
            fs::write(&path, lines.join("\n") + "\n").unwrap();
            scans.push((
                HostUser {
                    name: user.to_string(),
                    uid: None,
                    home: PathBuf::from("/home").join(user),
                },
                LogSet {
                    claude_sessions: vec![path],
                    ..LogSet::default()
                },
            ));
        }
        let range = Range {
            since: DateTime::from_timestamp(0, 0).unwrap(),
            until: DateTime::from_timestamp(4_000_000_000, 0).unwrap(),
        };
        build_report(&scans, crate::fun::Scope::AllUsers, range, 42)
    }

    fn prompt(id: usize, text: &str) -> String {
        json!({"type": "user", "uuid": format!("p{id}"),
               "timestamp": "2026-10-03T23:30:00Z", "cwd": "/home/alice/acme-corp",
               "gitBranch": "feature/project-falcon",
               "message": {"content": text}})
        .to_string()
    }

    fn all_formats(report: &FunReport) -> Vec<String> {
        vec![
            render_text(report, 5),
            render_markdown(report, 5),
            render_html(report, 5, &HtmlOptions::default()),
            render_discord(report, 5),
            render_json(report).unwrap(),
        ]
    }

    #[test]
    fn multiple_hosts_and_mixed_permissions_are_visible_without_identities() {
        let mut report = sample_report("multi-host", &[("alice", vec![prompt(1, "hello")])]);
        report.coverage.hosts_scanned = 3;
        report.coverage.hosts_all_users = 2;
        report.scope = Scope::MixedUsers;
        for output in [
            render_text(&report, 5),
            render_markdown(&report, 5),
            render_html(&report, 5, &HtmlOptions::default()),
            render_discord(&report, 5),
        ] {
            assert!(output.contains("3 hosts"));
            assert!(output.contains("All users where permitted"));
            assert!(!output.contains("alice"));
        }
    }

    #[test]
    fn no_log_content_reaches_any_output_format() {
        const SECRETS: &[&str] = &[
            "alice",
            "secret-client",
            "falcon-portal",
            "ultra-private-customer-dashboard",
            "sk-ant-api03-SUPERSECRET",
            "hunter2",
            "acme-corp",
            "project-falcon",
            "private_server",
            "customer-deploy",
            "falcon-finetune",
            "db.internal.example",
            "john",
        ];
        let lines = vec![
            prompt(1, "wtf my password is hunter2 why did you do that"),
            prompt(2, "fuck, the key sk-ant-api03-SUPERSECRET is wrong"),
            prompt(3, "you are useless, john said so"),
            prompt(4, "ssh to db.internal.example please, thanks"),
            prompt(
                5,
                "deploy to https://secret-client.xyz and falcon-portal.dk now!!!",
            ),
            json!({"type": "user", "uuid": "p6", "timestamp": "2026-10-03T23:30:09Z",
                   "cwd": "/srv/ultra-private-customer-dashboard",
                   "message": {"content": "misunderstanding"}})
            .to_string(),
            json!({"type": "assistant", "uuid": "a1",
            "timestamp": "2026-10-03T23:31:00Z",
            "message": {"id": "m1", "model": "falcon-finetune",
                "usage": {"input_tokens": 1, "output_tokens": 1},
                "content": [
                  {"type": "text", "text": "You're absolutely right! hunter2 is bad"},
                  {"type": "tool_use", "name": "mcp__private_server__query", "input": {}},
                  {"type": "tool_use", "name": "Bash",
                   "input": {"command": "./customer-deploy.sh && rm -rf /home/alice"}}
                ]}})
            .to_string(),
        ];
        let report = sample_report("privacy", &[("alice", lines)]);
        assert!(report.total.counts.prompts >= 4);
        assert!(report.total.total(Cat::Swear) > 0);

        for output in all_formats(&report) {
            for secret in SECRETS {
                assert!(!output.contains(secret), "{secret} leaked into:\n{output}");
            }
        }
    }

    #[test]
    fn html_contains_the_shared_stats_and_respects_top() {
        let report = sample_report(
            "html",
            &[("alice", vec![prompt(1, "please fix this 🚀 🚀 🙂")])],
        );
        let html = render_html(&report, 1, &HtmlOptions::default());
        assert!(html.starts_with("<!DOCTYPE html>\n<html lang=\"en\">"));
        assert!(html.ends_with("</html>\n"));
        assert!(html.contains("<meta charset=\"utf-8\">"));
        assert!(html.contains("<style>"));
        for forbidden in ["<script", "<link", "<img", "<iframe", "@import", "url(http"] {
            assert!(
                !html.contains(forbidden),
                "external resource or script: {forbidden}"
            );
        }
        for section in sections(&report, Style::Plain, 1) {
            assert!(html.contains(&escape_html(&section.title)));
            for line in section.chart.iter().chain(&section.lines) {
                let fragments = line.split_once(" — ").or_else(|| line.split_once(": "));
                if let Some((label, text)) = fragments {
                    assert!(html.contains(&escape_html(label)), "missing label: {label}");
                    assert!(html.contains(&escape_html(text)), "missing stat: {text}");
                } else {
                    assert!(html.contains(&escape_html(line)), "missing stat: {line}");
                }
            }
        }
        assert!(html.contains("🚀 ×2"));
        assert!(!html.contains("🙂 ×1"));
        assert!(render_html(&report, 2, &HtmlOptions::default()).contains("🙂 ×1"));
        assert!(html.contains(&escape_html(&footer(&report))));
    }

    #[test]
    fn html_escapes_report_strings_instead_of_interpreting_markup() {
        let mut report = sample_report(
            "html-escaping",
            &[
                ("alice", vec![prompt(1, "please fix this")]),
                ("bob", vec![prompt(2, "thanks")]),
            ],
        );
        // Exercise the renderer independently of the upstream privacy filter.
        let markup = "</li><script>alert(\"x\" & 'y')</script>";
        for user in &mut report.by_user {
            user.name = markup.to_string();
        }
        report.quotes = vec![markup.to_string()];
        let html = render_html(&report, 5, &HtmlOptions::default());
        assert!(!html.contains(markup));
        assert!(!html.contains("<script>"));
        assert!(html.contains(
            "&lt;/li&gt;&lt;script&gt;alert(&quot;x&quot; &amp; &#39;y&#39;)&lt;/script&gt;"
        ));
    }

    #[test]
    fn html_empty_report_is_a_complete_page() {
        let report = sample_report("html-empty", &[]);
        let html = render_html(&report, 5, &HtmlOptions::default());
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("No Claude Code or Codex sessions found in that period."));
        assert!(html.ends_with("</html>\n"));
        assert!(!html.contains("<nav"));
    }

    #[test]
    fn html_social_metadata_uses_public_urls_and_embedded_fonts() {
        let report = sample_report("social", &[("alice", vec![prompt(1, "wtf please")])]);
        let options = HtmlOptions::new(Some("https://example.com/stats/?a=1&b=2#old")).unwrap();
        let html = render_html(&report, 5, &options);
        assert!(
            html.contains("property=\"og:url\" content=\"https://example.com/stats/?a=1&amp;b=2\"")
        );
        assert!(html.contains(
            "property=\"og:image\" content=\"https://example.com/stats/ai-stats-social-v1.png\""
        ));
        assert!(html.contains("name=\"twitter:card\" content=\"summary_large_image\""));
        assert!(html.contains("1 prompt. 0 tokens. $0.25 in the swear jar."));
        assert_eq!(html.matches("data:font/woff2;base64,").count(), 3);
        assert!(html.contains("SIL OPEN FONT LICENSE Version 1.1"));
        assert!(!html.contains("src=\"http"));
        assert!(!html.contains("<script"));
        // The crawler can find the small metadata block before embedded fonts.
        assert!(html.find("og:image").unwrap() < html.find("@font-face").unwrap());
        let standalone = render_html(&report, 5, &HtmlOptions::default());
        assert!(!standalone.contains("property=\"og:image\""));
        assert!(!standalone.contains("property=\"og:url\""));
    }

    #[test]
    fn calendar_span_counts_both_endpoints() {
        let mut report = sample_report("calendar-span", &[]);
        for (first, last, expected) in [
            ((2026, 9, 8), (2026, 10, 31), "54 days"),
            ((2026, 10, 31), (2026, 10, 31), "1 day"),
            ((2024, 2, 28), (2024, 3, 1), "3 days"),
        ] {
            let date = |(y, m, d)| NaiveDate::from_ymd_opt(y, m, d).unwrap();
            report.total.by_day = BTreeMap::from([(date(first), 1), (date(last), 1)]);
            assert!(header(&report, Style::Plain)[0].contains(&format!(" · {expected} · ")));
        }
    }

    #[test]
    fn paper_estimates_use_full_prompt_characters_and_round_up_sheets() {
        let mut report = sample_report("paper", &[]);
        assert!(paper_trail(&report, Style::Plain).is_empty());
        report.total.counts.prompt_chars = 3_000_001;
        // The prose word count is intentionally unrelated: long pastes are
        // truncated for phrase analysis, but their full length still counts.
        report.total.counts.prompt_words = 2;
        let text = paper_trail(&report, Style::Plain).lines.join("\n");
        assert!(text.contains("about 1000.0 pages from 3,000,001 characters"));
        assert!(text.contains("1,001 sheets, 4.99 kg of paper, stacked 10.0 cm"));
        assert!(text.contains("297.3 metres"));
        assert!(text.contains("5.56 novels"));
        assert!(text.contains("33h 20m"));
        assert!(text.contains("~0.13 standard-tree equivalents"));
        assert!(text.contains("not measured trees saved or AI carbon savings"));
        report.total.counts.prompt_chars = 1;
        let text = paper_trail(&report, Style::Plain).lines.join("\n");
        assert!(text.contains("1 sheet, 5 g"));
        assert!(text.contains("0.01% of a standard tree"));
    }

    #[test]
    fn slurs_are_counted_but_never_printed_or_attributed() {
        let lines: Vec<String> = crate::fun::lexicon::slur_phrases()
            .enumerate()
            .map(|(index, phrase)| prompt(index, &format!("you are a {phrase}")))
            .collect();
        let report = sample_report(
            "slurs",
            &[("alice", lines), ("bob", vec![prompt(9999, "hello there")])],
        );

        assert_eq!(
            report.total.total(Cat::Slur),
            crate::fun::lexicon::slur_phrases().count() as u64
        );
        for tally in report
            .by_user
            .iter()
            .map(|user| &user.tally)
            .chain(report.by_tool.values())
        {
            assert_eq!(tally.total(Cat::Slur), 0);
        }

        for output in all_formats(&report) {
            let tokens = format!(" {} ", tokenize(&output).join(" "));
            for phrase in crate::fun::lexicon::slur_phrases() {
                let printed = format!(" {phrase} ");
                assert!(!tokens.contains(&printed), "a slur reached the output");
            }
        }
    }

    #[test]
    fn slur_detector_reports_a_clean_host() {
        let report = sample_report("clean", &[("alice", vec![prompt(1, "hello there")])]);
        assert!(render_text(&report, 5).contains("Slur detector: 0 hits in 1 prompt."));
    }

    #[test]
    fn names_are_roasted_host_wide_and_masked() {
        let report = sample_report(
            "names",
            &[
                (
                    "alice",
                    vec![
                        prompt(1, "ship https://secret-client.xyz now"),
                        json!({"type": "user", "uuid": "p2",
                               "timestamp": "2026-10-03T23:30:09Z",
                               "cwd": "/srv/falcon-final",
                               "message": {"content": "hello"}})
                        .to_string(),
                    ],
                ),
                ("bob", vec![prompt(3, "hello")]),
            ],
        );
        let text = render_text(&report, 5);
        assert!(text.contains("Worst project name: f••••-final"));
        assert!(text.contains("Worst domain: s••••.xyz"));
        assert!(render_json(&report).unwrap().contains("s••••.xyz"));
        assert!(!text.contains("falcon"));
        assert!(!text.contains("secret"));
    }

    #[test]
    fn longest_words_and_punctuation_are_reported() {
        let report = sample_report(
            "shape",
            &[("alice", vec![prompt(1, "misunderstanding!!!!! really???")])],
        );
        let text = render_text(&report, 5);
        assert!(text.contains("dropped a 16-letter word"));
        assert!(text.contains("5 punctuation marks in a row"));
        assert!(text.contains("Longest word: 16 letters, starts with m"));
        assert!(!text.contains("misunderstanding"));
    }

    #[test]
    fn quotes_are_rendered_masked() {
        let report = sample_report(
            "quotes",
            &[("alice", vec![prompt(1, "why the fuck did you do that")])],
        );
        assert_eq!(report.quotes, ["why the f**k did you do that"]);
        assert!(render_markdown(&report, 5).contains(r#""why the f\*\*k did you do that""#));
        assert!(render_text(&report, 5).contains(r#""why the f**k did you do that""#));
    }

    #[test]
    fn discord_messages_fit_the_limit() {
        let lines: Vec<String> = (0..200)
            .map(|i| {
                prompt(
                    i,
                    "wtf please fix docker nginx postgres redis rust python, this sucks, \
                     thanks :) still broken why did you do that lol sorry good morning",
                )
            })
            .collect();
        let users: Vec<(&str, Vec<String>)> = vec![("alice", lines.clone()), ("bob", lines)];
        let report = sample_report("discord", &users);
        let messages = discord_messages(&report, 25);
        assert!(!messages.is_empty());
        for message in &messages {
            assert!(message.chars().count() <= DISCORD_LIMIT);
        }
    }

    #[test]
    fn oversized_blocks_split_on_line_boundaries() {
        let long_block = (0..50)
            .map(|i| format!("line {i} ").repeat(5))
            .collect::<Vec<_>>()
            .join("\n");
        let messages = pack(vec!["short".to_string(), long_block], 200);
        assert!(messages.len() > 1);
        assert!(messages.iter().all(|m| m.chars().count() <= 200));
        assert!(messages[0].starts_with("short"));
    }

    #[test]
    fn single_user_runs_say_you_not_a_pseudonym() {
        let report = sample_report(
            "solo",
            &[("alice", vec![prompt(1, "this is a perfect plan")])],
        );
        let text = render_text(&report, 5);
        assert!(text.contains("You:"));
        assert!(!text.contains(&report.by_user[0].name));
    }

    #[test]
    fn multi_user_runs_attribute_awards_to_pseudonyms() {
        let report = sample_report(
            "multi",
            &[
                ("alice", vec![prompt(1, "this is perfect")]),
                ("bob", vec![prompt(2, "wtf")]),
            ],
        );
        let text = render_text(&report, 5);
        let swearer = report
            .by_user
            .iter()
            .find(|user| user.tally.total(Cat::Swear) > 0)
            .unwrap();
        assert!(text.contains(&swearer.name));
        assert!(!text.contains("You:"));
    }

    #[test]
    fn rates_ignore_users_with_too_few_prompts() {
        let report = sample_report(
            "rates",
            &[("alice", vec![prompt(1, "is this a question?")])],
        );
        assert!(!render_text(&report, 5).contains("Interrogator"));
    }

    #[test]
    fn empty_sections_are_left_out() {
        let report = sample_report("quiet", &[("alice", vec![prompt(1, "hmm")])]);
        let text = render_text(&report, 5);
        assert!(!text.contains("Danger Zone"));
        assert!(!text.contains("Claude vs Codex"));
    }

    #[test]
    fn number_helpers() {
        assert_eq!(fmt_compact(999), "999");
        assert_eq!(fmt_compact(1_500), "1.5K");
        assert_eq!(fmt_compact(4_200_000_000), "4.2B");
        assert_eq!(fmt_duration(42), "42s");
        assert_eq!(fmt_duration(120), "2m");
        assert_eq!(fmt_duration(3720), "1h 02m");
        let day = |d: u32| NaiveDate::from_ymd_opt(2026, 10, d).unwrap();
        let days = BTreeMap::from([(day(1), 1), (day(2), 1), (day(3), 1), (day(9), 1)]);
        assert_eq!(longest_streak(&days), 3);
    }

    #[test]
    fn serializes_to_json_with_named_categories() {
        let report = sample_report("json", &[("alice", vec![prompt(1, "wtf 🚀")])]);
        let value: serde_json::Value =
            serde_json::from_str(&render_json(&report).unwrap()).unwrap();
        assert_eq!(value["total"]["phrases"]["swear"]["wtf"], 1);
        assert_eq!(value["total"]["human_emoji"]["🚀"], 1);
        assert_eq!(value["total"]["prompts"], 1);
    }
}
