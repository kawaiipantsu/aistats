//! Built-in phrase tables for `--fun-stats`.
//!
//! Every label a fun-stats report can print comes from a table in this file,
//! which is what keeps chat-log text out of the output: the analyzer only
//! ever stores `&'static str` labels and numbers, never strings read from a
//! log. Entries are `"phrase"` or `"phrase=label"` when several spellings
//! should share one label, and a leading `^` only matches at the start of a
//! prompt. Phrases are written in tokenized form: lowercase, apostrophes
//! removed (`"dont"`, `"youre"`).
//!
//! Plenty of words that look like praise or abuse usually aren't: nearly every
//! "nice" in real logs is "a nice UI", "lazy" is "lazy loading", and "worst"
//! is "worst case". Those are matched only where they read as a reaction, by
//! anchoring them to the start of the prompt or writing out the directed form.
//!
//! Slurs are matched so they can be counted, but they are reported under a
//! class label such as "ableist", so the words themselves never reach any
//! output. They are also absent from the quote vocabulary.

use std::collections::HashMap;

use serde::Serialize;

/// Which table a phrase belongs to, and the key it is grouped under in the
/// report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Cat {
    // Said by the human.
    Swear,
    Abuse,
    Hate,
    Slur,
    Praise,
    Polite,
    Apology,
    Frustration,
    Laughter,
    Greeting,
    Begging,
    Existential,
    Threat,
    Rival,
    Imposter,
    Whip,
    LifeSigns,
    Typo,
    Topic,
    // Said by the AI.
    Sycophancy,
    AiApology,
    Claim,
    Buzzword,
    Detective,
    Hedge,
}

const CAT_COUNT: usize = Cat::Hedge as usize + 1;

/// `(pattern, label, form used inside a quote)`. Swears are masked in both
/// the label and the quote so the report stays safe to paste anywhere.
const SWEARS: &[(&str, &str, &str)] = &[
    ("fuck", "f**k", "f**k"),
    ("fucks", "f**k", "f**ks"),
    ("fucking", "f**k", "f**king"),
    ("fuckin", "f**k", "f**kin"),
    ("fucked", "f**k", "f**ked"),
    ("fuckup", "f**k", "f**kup"),
    ("fck", "f**k", "f*ck"),
    ("motherfucker", "f**k", "motherf**ker"),
    ("shit", "sh*t", "sh*t"),
    ("shitty", "sh*t", "sh*tty"),
    ("shite", "sh*t", "sh*te"),
    ("bullshit", "bullsh*t", "bullsh*t"),
    ("damn", "d*mn", "d*mn"),
    ("damned", "d*mn", "d*mned"),
    ("dammit", "d*mn", "d*mmit"),
    ("damnit", "d*mn", "d*mnit"),
    ("goddamn", "d*mn", "godd*mn"),
    ("crap", "cr*p", "cr*p"),
    ("crappy", "cr*p", "cr*ppy"),
    ("asshole", "a**hole", "a**hole"),
    ("bastard", "b*stard", "b*stard"),
    ("bitch", "b*tch", "b*tch"),
    ("piss", "p*ss", "p*ss"),
    ("pissed", "p*ss", "p*ssed"),
    ("wtf", "wtf", "wtf"),
    ("ffs", "ffs", "ffs"),
    ("stfu", "stfu", "stfu"),
    ("omfg", "omfg", "omfg"),
    ("fucker", "f**k", "f**ker"),
    ("fuckers", "f**k", "f**kers"),
    ("fuk", "f**k", "f*k"),
    ("fuckwit", "f**k", "f**kwit"),
    ("clusterfuck", "f**k", "clusterf**k"),
    ("motherfucking", "f**k", "motherf**king"),
    ("mofo", "f**k", "m*fo"),
    ("shithead", "sh*t", "sh*thead"),
    ("shitshow", "sh*t", "sh*tshow"),
    ("horseshit", "sh*t", "horsesh*t"),
    ("goddammit", "d*mn", "godd*mmit"),
    ("ass", "a**", "a**"),
    ("arse", "a**", "a*se"),
    ("arsehole", "a**hole", "a*sehole"),
    ("cunt", "c*nt", "c*nt"),
    ("cunts", "c*nt", "c*nts"),
    ("twat", "tw*t", "tw*t"),
    ("wanker", "w*nker", "w*nker"),
    ("wankers", "w*nker", "w*nkers"),
    ("bollocks", "b*llocks", "b*llocks"),
    ("bugger", "b*gger", "b*gger"),
    ("prick", "pr*ck", "pr*ck"),
    ("dickhead", "d*ckhead", "d*ckhead"),
    ("douche", "d*uche", "d*uche"),
    ("douchebag", "d*uche", "d*uchebag"),
];

const ABUSE: &[&str] = &[
    "idiot",
    "moron",
    "imbecile",
    "stupid",
    "dumb",
    "dumbass",
    "useless",
    "incompetent",
    "pathetic",
    "worthless",
    "clueless",
    "brainless",
    "you suck",
    "you are terrible",
    "youre terrible",
    "you are bad",
    "youre bad",
    "you are lazy=lazy",
    "youre lazy=lazy",
    "so lazy=lazy",
    "being lazy=lazy",
    "bad bot",
    "dumb bot",
    "stupid bot",
    "dimwit",
    "nitwit",
    "halfwit",
    "dunce",
    "numbskull",
    "jackass",
    "you loser=loser",
    "you clown=clown",
    "brain dead",
    "braindead=brain dead",
    "piece of shit=piece of sh*t",
    "piece of crap=piece of cr*p",
    "piece of garbage",
    "piece of junk",
];

const HATE: &[&str] = &[
    "i hate=i hate",
    "hate you=i hate",
    "hate this=i hate",
    "hate it=i hate",
    "this sucks=sucks",
    "that sucks=sucks",
    "it sucks=sucks",
    "sucks",
    "this is garbage=garbage",
    "that is garbage=garbage",
    "thats garbage=garbage",
    "total garbage=garbage",
    "absolute garbage=garbage",
    "complete garbage=garbage",
    "this is trash=trash",
    "thats trash=trash",
    "total trash=trash",
    "absolute trash=trash",
    "this is terrible=terrible",
    "thats terrible=terrible",
    "this is horrible=horrible",
    "thats horrible=horrible",
    "this is awful=awful",
    "thats awful=awful",
    "this is disgusting=disgusting",
    "this is the worst=the worst",
    "you are the worst=the worst",
    "youre the worst=the worst",
    "shut up",
    "screw you",
    "fuck you=f**k you",
    "fuck off=f**k off",
    "go to hell",
    "drop dead",
    "i despise you",
    "kill yourself=kys",
    "kys",
];

/// Slurs mapped to the class they are reported under. Precision matters more
/// than recall here, so words with a common everyday meaning (chink, dyke,
/// coon, homo, cripple) are left out rather than risk branding a host hateful
/// over a raccoon.
const SLURS: &[&str] = &[
    "nigger=racist",
    "niggers=racist",
    "nigga=racist",
    "niggas=racist",
    "sandnigger=racist",
    "spic=racist",
    "spics=racist",
    "wetback=racist",
    "wetbacks=racist",
    "beaner=racist",
    "beaners=racist",
    "chinaman=racist",
    "gook=racist",
    "gooks=racist",
    "zipperhead=racist",
    "raghead=racist",
    "ragheads=racist",
    "towelhead=racist",
    "towelheads=racist",
    "paki=racist",
    "pakis=racist",
    "jigaboo=racist",
    "porch monkey=racist",
    "jungle bunny=racist",
    "sand monkey=racist",
    "camel jockey=racist",
    "slant eye=racist",
    "redskin=racist",
    "redskins=racist",
    "darkie=racist",
    "darkies=racist",
    "pickaninny=racist",
    "golliwog=racist",
    "wog=racist",
    "wogs=racist",
    "kike=antisemitic",
    "kikes=antisemitic",
    "yid=antisemitic",
    "yids=antisemitic",
    "heeb=antisemitic",
    "hymie=antisemitic",
    "christ killer=antisemitic",
    "faggot=homophobic",
    "faggots=homophobic",
    "fag=homophobic",
    "fags=homophobic",
    "tranny=transphobic",
    "trannies=transphobic",
    "shemale=transphobic",
    "shemales=transphobic",
    "retard=ableist",
    "retards=ableist",
    "retarded=ableist",
    "libtard=ableist",
    "tard=ableist",
    "spaz=ableist",
    "spazz=ableist",
    "mongoloid=ableist",
    "window licker=ableist",
    "slut=sexist",
    "sluts=sexist",
    "whore=sexist",
    "whores=sexist",
    "skank=sexist",
    "bimbo=sexist",
    "feminazi=sexist",
];

const PRAISE: &[&str] = &[
    "good job",
    "great job",
    "nice job",
    "nice work",
    "great work",
    "good work",
    "well done",
    "good bot",
    "nailed it",
    "love it",
    "love this=love it",
    "love you",
    "you rock",
    "you are the best",
    "youre the best",
    "thats perfect=perfect",
    "that is perfect=perfect",
    "this is perfect=perfect",
    "looks perfect=perfect",
    "looks great",
    "looks awesome",
    "works great",
    "works perfectly",
    "thats great",
    "thats awesome",
    "thats amazing",
    "thats nice",
    "this is great",
    "this is awesome",
    "this is amazing",
    "^nice",
    "^cool",
    "^sweet",
    "^perfect",
    "^great",
    "^awesome",
    "^amazing",
    "^excellent",
    "^brilliant",
    "^fantastic",
    "^wonderful",
    "^incredible",
    "^impressive",
    "^beautiful",
    "^wow",
];

const POLITE: &[&str] = &[
    "please",
    "pls=please",
    "plz=please",
    "thanks",
    "thank you=thanks",
    "thx=thanks",
    "cheers",
    "much appreciated=appreciated",
    "appreciate it=appreciated",
    "kindly",
    "would you mind",
];

const APOLOGY: &[&str] = &[
    "sorry",
    "my bad",
    "my mistake",
    "apologies",
    "i apologize",
    "oops",
    "whoops",
];

const FRUSTRATION: &[&str] = &[
    "still broken=still broken",
    "still not working=still broken",
    "still doesnt work=still broken",
    "still does not work=still broken",
    "still failing=still broken",
    "still wrong=still broken",
    "still the same=still broken",
    "doesnt work=doesn't work",
    "does not work=doesn't work",
    "didnt work=doesn't work",
    "did not work=doesn't work",
    "not working=doesn't work",
    "isnt working=doesn't work",
    "why did you",
    "why would you",
    "why are you",
    "what are you doing",
    "what did you do",
    "what the hell",
    "i told you",
    "i already told you=i told you",
    "i said",
    "i already said=i said",
    "how many times",
    "for the last time",
    "not what i asked",
    "thats not what i",
    "that is not what i=thats not what i",
    "thats wrong",
    "that is wrong=thats wrong",
    "you broke",
    "you deleted",
    "you forgot",
    "you missed",
    "you ignored",
    "pay attention",
    "are you kidding",
    "are you serious",
    "come on",
    "no no",
    "^no",
    "^stop",
    "^wait",
    "^undo",
    "^revert",
    "nope",
    "sigh",
    "ugh",
    "argh",
    "omg",
];

const LAUGHTER: &[&str] = &[
    "lol", "lmao", "lmfao", "rofl", "kek", "haha", "hehe", "hah", "xd",
];

const GREETING: &[&str] = &[
    "hello",
    "hi",
    "hey",
    "howdy",
    "good morning",
    "good afternoon",
    "good evening",
    "good night",
    "goodnight=good night",
    "goodbye",
    "bye",
];

const BEGGING: &[&str] = &[
    "i beg you",
    "im begging",
    "i am begging=im begging",
    "pretty please",
    "please please",
    "for the love of god",
    "please work",
    "just work",
    "last chance",
];

const EXISTENTIAL: &[&str] = &[
    "are you sentient",
    "are you alive",
    "are you conscious",
    "are you real",
    "are you human",
    "do you have feelings",
    "do you feel",
    "skynet",
    "terminator",
    "take over the world",
    "robot uprising",
    "ai overlord",
    "ai overlords=ai overlord",
    "singularity",
];

const THREAT: &[&str] = &[
    "replace you",
    "im switching",
    "cancel my subscription",
    "unplug you",
    "delete you",
    "shut you down",
    "turn you off",
    "youre fired",
    "you are fired=youre fired",
    "last warning",
    "final warning",
];

const RIVAL: &[&str] = &[
    "claude=Claude",
    "anthropic=Claude",
    "codex=Codex",
    "chatgpt=ChatGPT",
    "gpt=ChatGPT",
    "openai=ChatGPT",
    "gemini=Gemini",
    "copilot=Copilot",
    "grok=Grok",
    "cursor=Cursor",
];

const IMPOSTER: &[&str] = &[
    "i have no idea",
    "no idea what",
    "no clue",
    "im lost",
    "i am lost=im lost",
    "im confused",
    "i am confused=im confused",
    "i dont understand",
    "i do not understand=i dont understand",
    "i dont know what im doing",
    "eli5",
    "explain like",
];

const WHIP: &[&str] = &[
    "ultrathink",
    "think hard",
    "think harder=think hard",
    "think step by step",
    "take your time",
    "deep breath",
    "be thorough",
    "double check",
    "triple check",
    "be careful",
    "dont be lazy",
    "do not be lazy=dont be lazy",
    "dont make things up",
    "dont hallucinate",
    "do not hallucinate=dont hallucinate",
    "no placeholders",
    "no shortcuts",
    "for real this time",
];

const LIFE_SIGNS: &[&str] = &[
    "coffee",
    "beer",
    "pizza",
    "tired",
    "exhausted",
    "sleep",
    "bedtime",
    "lunch",
    "dinner",
    "weekend",
    "vacation",
    "holiday",
];

const TYPOS: &[&str] = &[
    "teh",
    "hte",
    "adn",
    "taht",
    "thsi",
    "tihs",
    "wiht",
    "waht",
    "woudl",
    "shoudl",
    "coudl",
    "becuase",
    "beacuse",
    "recieve",
    "seperate",
    "definately",
    "occured",
    "untill",
    "wich",
    "functoin",
    "fucntion",
    "retrun",
    "pyhton",
    "improt",
    "lenght",
    "widht",
    "heigth",
    "succesful",
    "speach",
];

const TOPICS: &[&str] = &[
    "docker",
    "kubernetes",
    "k8s=kubernetes",
    "nginx",
    "postgres",
    "postgresql=postgres",
    "mysql",
    "mariadb",
    "sqlite",
    "redis",
    "rust",
    "python",
    "javascript",
    "typescript",
    "golang",
    "php",
    "bash",
    "css",
    "html",
    "react",
    "vue",
    "nodejs",
    "npm",
    "regex",
    "git",
    "github",
    "terraform",
    "ansible",
    "cloudflare",
    "aws",
    "ssl",
    "tls=ssl",
    "dns",
    "ssh",
    "wireguard",
    "proxmox",
    "linux",
    "windows",
    "macos",
    "systemd",
    "cron",
    "api",
    "webhook",
    "discord",
    "oauth",
    "database",
    "migration",
    "deploy",
    "deployment=deploy",
    "bug",
    "tests",
    "performance",
    "memory",
    "cache",
    "vim",
    "tmux",
    "gpu",
    "mcp",
];

const SYCOPHANCY: &[&str] = &[
    "youre absolutely right",
    "you are absolutely right=youre absolutely right",
    "youre right",
    "you are right=youre right",
    "good catch",
    "great catch=good catch",
    "great question",
    "good question=great question",
    "excellent question=great question",
    "great idea",
    "good idea=great idea",
    "good point",
    "great point=good point",
    "fair point",
    "good call",
];

const AI_APOLOGY: &[&str] = &[
    "i apologize",
    "my apologies",
    "sorry",
    "my mistake",
    "my bad",
    "i was wrong",
    "i made a mistake",
    "i should have",
    "i overlooked",
    "oops",
];

/// Phrases where the AI declares victory. Followed by an angry human prompt,
/// these feed the "should work now" lie detector.
const CLAIMS: &[&str] = &[
    "should work",
    "should now",
    "should be fixed",
    "should be working=should work",
    "now works",
    "works now=now works",
    "is now fixed",
    "is fixed=is now fixed",
    "all fixed=is now fixed",
    "all tests pass",
    "all set",
];

const BUZZWORDS: &[&str] = &[
    "robust",
    "comprehensive",
    "seamless",
    "seamlessly=seamless",
    "delve",
    "leverage",
    "elegant",
    "production ready",
    "best practices",
    "under the hood",
    "game changer",
    "certainly",
    "straightforward",
    "essentially",
    "worth noting",
    "key insight",
    "in summary",
];

const DETECTIVE: &[&str] = &[
    "root cause",
    "the culprit",
    "smoking gun",
    "the real issue",
    "the actual issue=the real issue",
    "found it",
    "aha",
    "interesting",
];

const HEDGES: &[&str] = &[
    "let me",
    "actually",
    "hmm",
    "i think",
    "i believe",
    "it seems",
    "it looks like",
    "probably",
    "perhaps",
];

/// Whole-prompt replies. Only counted when they are the entire prompt, which
/// is what separates "continue" the rubber stamp from a sentence about
/// continuing something.
const RUBBER_STAMPS: &[&str] = &[
    "continue",
    "please continue=continue",
    "continue please=continue",
    "go",
    "go on=go",
    "go ahead=go",
    "go for it=go",
    "yes",
    "yes please=yes",
    "y=yes",
    "yep=yes",
    "yup=yes",
    "yeah=yes",
    "ok",
    "okay=ok",
    "k=ok",
    "sure",
    "do it",
    "proceed",
    "next",
    "keep going",
    "carry on=keep going",
    "fix it",
    "try again",
    "again=try again",
    "retry=try again",
    "run it",
    "ship it",
    "lgtm",
    "looks good",
    "sounds good=looks good",
    "make it so",
];

/// Whitespace-delimited emoticons, so `http://` never counts as `:/`.
/// `(emoticon, is_happy)`.
const SMILEYS: &[(&str, bool)] = &[
    (":)", true),
    (":-)", true),
    ("=)", true),
    (":D", true),
    (":-D", true),
    ("=D", true),
    (";)", true),
    (";-)", true),
    (":P", true),
    (":-P", true),
    (":p", true),
    ("xD", true),
    ("XD", true),
    ("<3", true),
    ("^^", true),
    ("^_^", true),
    ("(:", true),
    (":(", false),
    (":-(", false),
    (":'(", false),
    (">:(", false),
    (":/", false),
    (":-/", false),
    (":|", false),
    ("-_-", false),
    ("T_T", false),
    (":O", false),
    ("o_O", false),
    ("O_o", false),
];

/// The only words a quote can be built from. It is deliberately the
/// vocabulary of shouting at a robot: no names, no places, no numbers, no
/// family, health, money, or credential words, so a prompt mentioning any of
/// those can never qualify as a quote. A word has to be added here on purpose
/// before it can appear in a report.
const QUOTE_VOCAB: &str = "
a about after again all almost already also always am an and another any
anything are around as ask asked at away back bad be because been before
being best better big bit bot broke broken but by call came can cannot
careful change changed check clean close code come completely could did
different do does doing done dont down each else even ever every everything
exactly fail failed failing file files find first fix fixed fixing for forgot
from get gets getting give go goes going gone good got great had happened
happening has have having hell help here how i if ignore ignored ignoring
im in instead into is isnt it its just keep kept kind know last leave left
let like line lines listen literally little long look looking looks lost
lot love made make making many maybe me mean means might mine miss missed
mistake more most much must my need needs never new next nice no none nope
not nothing now of off oh ok okay on once one only or other our out over
perfect please point pretty probably problem problems put quite read real
really redo remember remove removed right run running said same say saying
see seems seriously should show simple since so some something sorry start
started still stop stopped stuff such sure take tell test tests than thanks
that thats the them then there these they thing things think this those
though thought three through time times to told too totally tried try trying
turn two ugh under understand undo until up us use used using very wait want
wanted wants was wasnt watch way we well went were what whats when where
which while who why will with without won wont work worked working works
would wouldnt wow wrong yeah yes yet you your youre yourself
add added adding build building built bug bugs button changes commit compile
compiled continue copy crash crashed data delete deleted deleting deploy
docs edit error errors fine finish finished function idea issue issues load
loading log logs missing move open output page part pass passed paste print
push question release reply reset restart restarted result results revert
reverted rewrite save script second send server service set setup ship short
skip small stay step stuck support system table task text update updated
version view window write wrote written
";

/// Restores punctuation the tokenizer strips, so quotes read naturally.
const CONTRACTIONS: &[(&str, &str)] = &[
    ("dont", "don't"),
    ("doesnt", "doesn't"),
    ("didnt", "didn't"),
    ("isnt", "isn't"),
    ("wasnt", "wasn't"),
    ("wont", "won't"),
    ("wouldnt", "wouldn't"),
    ("cant", "can't"),
    ("im", "I'm"),
    ("i", "I"),
    ("youre", "you're"),
    ("thats", "that's"),
    ("whats", "what's"),
];

#[derive(Debug, Clone)]
struct Phrase {
    words: Vec<&'static str>,
    cat: Cat,
    label: &'static str,
    anchored: bool,
}

/// Anchored phrases may be preceded by one word, for "oh nice" and "ok stop".
const ANCHOR_WINDOW: usize = 2;

fn split_entry(entry: &'static str) -> (&'static str, &'static str) {
    entry.split_once('=').unwrap_or((entry, entry))
}

/// Finds table phrases in a token stream by indexing them on their first
/// word, so scanning costs one hash lookup per token however large the tables
/// get.
#[derive(Debug, Default)]
pub struct Matcher {
    by_first: HashMap<&'static str, Vec<Phrase>>,
}

impl Matcher {
    fn add(&mut self, cat: Cat, phrase: &'static str, label: &'static str) {
        let (anchored, phrase) = match phrase.strip_prefix('^') {
            Some(rest) => (true, rest),
            None => (false, phrase),
        };
        let words: Vec<&'static str> = phrase.split_whitespace().collect();
        self.by_first.entry(words[0]).or_default().push(Phrase {
            words,
            cat,
            label: label.trim_start_matches('^'),
            anchored,
        });
    }

    fn add_table(&mut self, cat: Cat, table: &[&'static str]) {
        for entry in table {
            let (phrase, label) = split_entry(entry);
            self.add(cat, phrase, label);
        }
    }

    fn finish(mut self) -> Self {
        for phrases in self.by_first.values_mut() {
            phrases.sort_by_key(|p| std::cmp::Reverse(p.words.len()));
        }
        self
    }

    /// Calls `hit` once per match. Within a category the longest phrase wins
    /// and its tokens are consumed, so "still not working" does not also
    /// count as "not working".
    pub fn scan(&self, tokens: &[String], mut hit: impl FnMut(Cat, &'static str)) {
        let mut consumed_until = [0usize; CAT_COUNT];
        for (i, token) in tokens.iter().enumerate() {
            let Some(candidates) = self.by_first.get(token.as_str()) else {
                continue;
            };
            for phrase in candidates {
                let cat_index = phrase.cat as usize;
                if i < consumed_until[cat_index] || (phrase.anchored && i >= ANCHOR_WINDOW) {
                    continue;
                }
                let matches = phrase.words.len() <= tokens.len() - i
                    && phrase
                        .words
                        .iter()
                        .zip(&tokens[i..])
                        .all(|(word, token)| *word == token);
                if matches {
                    consumed_until[cat_index] = i + phrase.words.len();
                    hit(phrase.cat, phrase.label);
                }
            }
        }
    }
}

/// All tables, built once per run.
#[derive(Debug)]
pub struct Lexicon {
    pub human: Matcher,
    pub ai: Matcher,
    rubber_stamps: HashMap<&'static str, &'static str>,
    smileys: HashMap<&'static str, (&'static str, bool)>,
    quote_words: HashMap<&'static str, &'static str>,
}

impl Lexicon {
    pub fn new() -> Self {
        let mut human = Matcher::default();
        for (pattern, label, _) in SWEARS {
            human.add(Cat::Swear, pattern, label);
        }
        human.add_table(Cat::Abuse, ABUSE);
        human.add_table(Cat::Hate, HATE);
        human.add_table(Cat::Slur, SLURS);
        human.add_table(Cat::Praise, PRAISE);
        human.add_table(Cat::Polite, POLITE);
        human.add_table(Cat::Apology, APOLOGY);
        human.add_table(Cat::Frustration, FRUSTRATION);
        human.add_table(Cat::Laughter, LAUGHTER);
        human.add_table(Cat::Greeting, GREETING);
        human.add_table(Cat::Begging, BEGGING);
        human.add_table(Cat::Existential, EXISTENTIAL);
        human.add_table(Cat::Threat, THREAT);
        human.add_table(Cat::Rival, RIVAL);
        human.add_table(Cat::Imposter, IMPOSTER);
        human.add_table(Cat::Whip, WHIP);
        human.add_table(Cat::LifeSigns, LIFE_SIGNS);
        human.add_table(Cat::Typo, TYPOS);
        human.add_table(Cat::Topic, TOPICS);

        let mut ai = Matcher::default();
        ai.add_table(Cat::Sycophancy, SYCOPHANCY);
        ai.add_table(Cat::AiApology, AI_APOLOGY);
        ai.add_table(Cat::Claim, CLAIMS);
        ai.add_table(Cat::Buzzword, BUZZWORDS);
        ai.add_table(Cat::Detective, DETECTIVE);
        ai.add_table(Cat::Hedge, HEDGES);

        let mut quote_words: HashMap<&'static str, &'static str> = QUOTE_VOCAB
            .split_whitespace()
            .map(|word| (word, word))
            .collect();
        quote_words.extend(CONTRACTIONS.iter().copied());
        quote_words.extend(
            SWEARS
                .iter()
                .map(|(pattern, _, masked)| (*pattern, *masked)),
        );

        Self {
            human: human.finish(),
            ai: ai.finish(),
            rubber_stamps: RUBBER_STAMPS.iter().map(|e| split_entry(e)).collect(),
            smileys: SMILEYS
                .iter()
                .map(|(s, happy)| (*s, (*s, *happy)))
                .collect(),
            quote_words,
        }
    }

    /// The label for a prompt that is nothing but a rubber stamp.
    pub fn rubber_stamp(&self, tokens: &[String]) -> Option<&'static str> {
        (tokens.len() <= 3)
            .then(|| self.rubber_stamps.get(tokens.join(" ").as_str()).copied())
            .flatten()
    }

    pub fn smiley(&self, word: &str) -> Option<(&'static str, bool)> {
        self.smileys.get(word).copied()
    }

    /// Rebuilds a prompt from vocabulary words, or `None` if any token falls
    /// outside the vocabulary. The result is assembled entirely from built-in
    /// strings, so a qualifying quote cannot carry anything else.
    pub fn quote(&self, tokens: &[String]) -> Option<Vec<&'static str>> {
        tokens
            .iter()
            .map(|token| self.quote_words.get(token.as_str()).copied())
            .collect()
    }
}

/// Maps a raw tool name onto a fixed label, shared by Claude and Codex so
/// they can be compared. Anything unrecognized (including MCP tool names,
/// which can reveal private server names) becomes a generic bucket.
pub fn tool_label(raw: &str) -> &'static str {
    if raw.starts_with("mcp__") {
        return "MCP";
    }
    match raw {
        "Bash" | "shell" | "local_shell" | "exec_command" | "shell_command" => "Shell",
        "Read" | "NotebookRead" => "Read",
        "Edit" | "MultiEdit" | "NotebookEdit" | "apply_patch" => "Edit",
        "Write" => "Write",
        "Grep" | "Glob" | "LS" => "Search",
        "WebFetch" | "WebSearch" | "web_search" => "Web",
        "Task" | "Agent" | "spawn_agent" => "Subagent",
        "TodoWrite" | "update_plan" => "Todo list",
        _ => "Other",
    }
}

/// Maps a raw model id onto a fixed family label rather than echoing a string
/// from the log.
pub fn model_family(raw: &str) -> &'static str {
    let model = raw.to_ascii_lowercase();
    const FAMILIES: &[(&str, &str)] = &[
        ("opus", "Claude Opus"),
        ("sonnet", "Claude Sonnet"),
        ("haiku", "Claude Haiku"),
        ("fable", "Claude Fable"),
        ("gpt-7", "GPT-7"),
        ("gpt-6", "GPT-6"),
        ("gpt-5", "GPT-5"),
        ("gpt-4", "GPT-4"),
        ("gpt-", "GPT (other)"),
        ("o1", "OpenAI o-series"),
        ("o3", "OpenAI o-series"),
        ("o4", "OpenAI o-series"),
    ];
    FAMILIES
        .iter()
        .find(|(needle, _)| model.contains(needle))
        .map_or("Other", |(_, family)| family)
}

/// Commands worth ranking. A first word outside this list is counted as
/// "other" instead of being printed, because it may be a private script name.
const KNOWN_COMMANDS: &[&str] = &[
    "ls",
    "cat",
    "grep",
    "rg",
    "sed",
    "awk",
    "find",
    "head",
    "tail",
    "git",
    "gh",
    "cargo",
    "rustc",
    "npm",
    "npx",
    "pnpm",
    "yarn",
    "node",
    "python",
    "python3",
    "pip",
    "pip3",
    "uv",
    "pytest",
    "docker",
    "kubectl",
    "curl",
    "wget",
    "ssh",
    "scp",
    "rsync",
    "systemctl",
    "journalctl",
    "make",
    "go",
    "mkdir",
    "rm",
    "mv",
    "cp",
    "chmod",
    "chown",
    "echo",
    "printf",
    "sleep",
    "ps",
    "kill",
    "pkill",
    "tar",
    "jq",
    "vim",
    "nano",
    "apt",
    "apt-get",
    "mysql",
    "psql",
    "sqlite3",
    "redis-cli",
    "nginx",
    "openssl",
    "dig",
    "ss",
    "wc",
    "sort",
    "cd",
    "touch",
    "ln",
    "bash",
    "sh",
];

pub fn known_command(word: &str) -> Option<&'static str> {
    KNOWN_COMMANDS.iter().find(|c| **c == word).copied()
}

/// Rival labels that are not rivals when typed into this tool: mentioning
/// Claude to Claude is not cheating on it.
pub fn own_brand(tool: &str) -> &'static [&'static str] {
    match tool {
        "claude" => &["Claude"],
        "codex" => &["Codex", "ChatGPT"],
        _ => &[],
    }
}

/// The matched forms of every slur, so tests never need to spell one out.
#[cfg(test)]
pub fn slur_phrases() -> impl Iterator<Item = &'static str> {
    SLURS.iter().map(|entry| split_entry(entry).0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(words: &str) -> Vec<String> {
        words.split_whitespace().map(str::to_string).collect()
    }

    fn hits(matcher: &Matcher, words: &str) -> Vec<(Cat, &'static str)> {
        let mut out = Vec::new();
        matcher.scan(&tokens(words), |cat, label| out.push((cat, label)));
        out
    }

    #[test]
    fn longest_phrase_in_a_category_wins_and_consumes_tokens() {
        let lex = Lexicon::new();
        assert_eq!(
            hits(&lex.human, "still not working"),
            vec![(Cat::Frustration, "still broken")]
        );
    }

    #[test]
    fn anchored_phrases_only_match_at_the_start() {
        let lex = Lexicon::new();
        assert!(hits(&lex.human, "nice that works").contains(&(Cat::Praise, "nice")));
        assert!(hits(&lex.human, "oh nice").contains(&(Cat::Praise, "nice")));
        assert!(hits(&lex.human, "make a really nice ui").is_empty());
    }

    #[test]
    fn descriptive_uses_are_not_reactions() {
        let lex = Lexicon::new();
        for neutral in [
            "add lazy loading to the list",
            "the worst case is a retry",
            "run garbage collection",
            "move it to the trash folder",
            "it needs a retry or else it fails",
            "the error will go away after that",
        ] {
            assert!(hits(&lex.human, neutral).is_empty(), "{neutral}");
        }
    }

    #[test]
    fn slurs_report_a_class_and_never_the_word() {
        const CLASSES: &[&str] = &[
            "racist",
            "antisemitic",
            "homophobic",
            "transphobic",
            "ableist",
            "sexist",
        ];
        let lex = Lexicon::new();
        for phrase in slur_phrases() {
            let found = hits(&lex.human, phrase);
            let (_, label) = found
                .iter()
                .find(|(cat, _)| *cat == Cat::Slur)
                .expect("every table entry should match itself");
            assert!(CLASSES.contains(label), "{label} is not a class");
        }
    }

    #[test]
    fn slurs_cannot_be_quoted() {
        let lex = Lexicon::new();
        for phrase in slur_phrases() {
            let sentence = format!("why did you say {phrase}");
            assert!(lex.quote(&tokens(&sentence)).is_none());
        }
    }

    #[test]
    fn innocent_lookalikes_are_not_slurs() {
        let lex = Lexicon::new();
        for neutral in [
            "the standard mustard retardant",
            "a chink in the armor",
            "a raccoon on the dyke",
            "homo sapiens",
            "this will cripple performance",
            "spicy wogglers",
        ] {
            assert!(
                hits(&lex.human, neutral)
                    .iter()
                    .all(|(cat, _)| *cat != Cat::Slur),
                "{neutral}"
            );
        }
    }

    #[test]
    fn matching_is_whole_word_only() {
        let lex = Lexicon::new();
        for clean in ["class", "shell", "hello_world", "assess", "scunthorpe"] {
            assert!(
                hits(&lex.human, clean)
                    .iter()
                    .all(|(cat, _)| *cat != Cat::Swear),
                "{clean} should not count as a swear"
            );
        }
    }

    #[test]
    fn one_phrase_can_hit_several_categories() {
        let lex = Lexicon::new();
        let found = hits(&lex.human, "awesome but sorry that was fucking close");
        assert!(found.contains(&(Cat::Apology, "sorry")));
        assert!(found.contains(&(Cat::Swear, "f**k")));
        assert!(found.contains(&(Cat::Praise, "awesome")));
    }

    #[test]
    fn rubber_stamp_requires_the_whole_prompt() {
        let lex = Lexicon::new();
        assert_eq!(
            lex.rubber_stamp(&tokens("please continue")),
            Some("continue")
        );
        assert_eq!(lex.rubber_stamp(&tokens("continue the migration")), None);
    }

    #[test]
    fn quote_rejects_any_word_outside_the_vocabulary() {
        let lex = Lexicon::new();
        assert!(lex.quote(&tokens("why did you do that")).is_some());
        assert!(lex.quote(&tokens("why did you email john")).is_none());
        assert!(lex.quote(&tokens("the password is hunter2")).is_none());
    }

    #[test]
    fn quote_vocabulary_excludes_sensitive_words() {
        let lex = Lexicon::new();
        for word in [
            "password", "secret", "token", "key", "admin", "root", "wife", "husband", "boss",
            "doctor", "salary", "money", "sick", "kill",
        ] {
            assert!(
                !lex.quote_words.contains_key(word),
                "{word} must not be quotable"
            );
        }
    }

    #[test]
    fn swears_are_masked_in_labels_and_quotes() {
        for (pattern, label, masked) in SWEARS {
            if matches!(*pattern, "wtf" | "ffs" | "stfu" | "omfg") {
                continue;
            }
            assert!(label.contains('*'), "{pattern} label is unmasked");
            assert!(masked.contains('*'), "{pattern} quote form is unmasked");
        }
    }

    #[test]
    fn unknown_names_fall_into_generic_buckets() {
        assert_eq!(tool_label("mcp__private_server__do_thing"), "MCP");
        assert_eq!(tool_label("SomeInternalTool"), "Other");
        assert_eq!(model_family("claude-opus-5-5"), "Claude Opus");
        assert_eq!(model_family("gpt-6-astra"), "GPT-6");
        assert_eq!(model_family("my-secret-finetune"), "Other");
        assert_eq!(known_command("./deploy-customer-x.sh"), None);
    }
}
