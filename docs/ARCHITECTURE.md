# ai-stats: Project-Specific Architecture

`ai-stats` is a single-binary Rust CLI, not a client/server web application,
with an embedded database and static report exporters:

```text
CLI (clap)
   |
   +-- commands/*        thin per-subcommand orchestration
   |
   +-- providers/*        one module per provider, all implementing the
   |                       UsageProvider trait (fetch_usage/test_connection/
   |                       list_models). openai.rs and anthropic.rs call the
   |                       real Admin/Usage APIs; ollama.rs and lmstudio.rs
   |                       read local model inventory only.
   |
   +-- import.rs           generic CSV/JSON ingestion for providers with no
   |                       programmatic usage API (Google, xAI, ...)
   |
   +-- db.rs (SQLite)      normalized local cache: usage_records,
   |                       cost_records, sync_state
   |
   +-- aggregate.rs        turns raw rows into overview/by-provider/
   |                       by-model/time-series rollups
   |
   +-- report/*            one renderer per output format (text, markdown,
   |                        json, csv, pdf, discord), all consuming the same
   |                        aggregate::Report so formats never disagree
   |
   +-- fun/*               --fun-stats: reads local Claude Code/Codex logs,
                            independent of the cache and provider APIs.
                            discover -> parse -> analyze -> FunReport, rendered
                            by report/fun.rs
```

Fun stats HTML uses the same section builders as text, Markdown, and Discord.
The HTML presentation lives in `report/fun/html.rs` and escapes text fragments
at the HTML boundary. It embeds `report/fun.css`, original SVG artwork, and
vendored OFL web fonts (with licenses) into a single offline document. A CSP
blocks scripts and network resources, allowing inline styles and only data-URL
fonts. Motion is CSS-only, with a pause control and reduced-motion handling.
`--public-url` adds canonical/OG/X metadata and writes a bundled PNG beside
`--output`; image URLs are absolute HTTP(S), never inline data. The public URL
is explicit, not inferred from scanned data or filesystem paths. Metadata uses
only aggregate counters. Image rendering is a development-time step; the binary
needs no browser, image tool, font server, or network to export a report.

**Local storage.** The SQLite cache lives at `~/.config/ai-stats/cache/usage.db`.
Configuration and credentials live in `config.yaml` under the same root, with
owner-only permissions on Unix. Environment variables take precedence over
stored credentials. No database server is required.

**Explicit network operations.** `sync` and provider connection tests call
provider APIs; model inventory queries contact the configured local servers; opt-in `--ssh-host` connects to the named machines to read logs.
`--send` with `--format discord`, on `report` or `--fun-stats`,
is the only way to send data to a third party (a user-configured webhook) —
rendering Discord output with `--format discord` alone never sends anything.

**SSH collection is explicit and ephemeral.** `fun::remote` sends the bundled
`remote.py` helper over stdin to `ssh ... python3 -`. It reads only known log
directories, with root/all-users or non-root/own-account scope. OpenSSH uses
batch authentication, strict host-key verification, no agent/port forwarding,
and a bounded runtime. The framed stream has bounded metadata and file totals;
locally generated directories prevent remote paths from escaping a private
`tempfile` snapshot. No archive extraction or remote shell interpolation is
needed. Snapshots are removed after scanning or on ordinary failures. Remote
identities are namespaced by SSH destination before pseudonym generation, while
the actual account/home remain available to the existing name-privacy filter.
The same analyzer handles all logs, all formats share the resulting totals,
and only host counts/scopes enter the public report. Any host failure aborts
export so scheduled publication retains the last complete report.

**`--fun-stats` keeps log text out of its output by type, not by redaction.**
Redaction can miss things, and this report is built to be posted publicly
after reading other people's chats as root. So `fun::analyze::Tally` has no
field that can hold a string from a log: only numbers, dates, emoji
characters, and `&'static str` labels from `fun::lexicon`. Model, tool, and
command names are mapped onto fixed labels. Quotes are `Vec<&'static str>`,
looked up word by word in a curated vocabulary. Longest-word stats keep a
length and initial. Project and domain names are judged where they are found
in `fun::names`, and only a masked verdict is kept. The only `String`s in a
report are generated pseudonyms, those masked verdicts, and rebuilt quotes. A
change that leaked log text would have to change these types, and
`report::fun`'s leak test covers every output format. Slurs in `fun::lexicon` go a step further: they are reported
only by class, and only as a host-wide total, because a count tied to a
pseudonym could point at a person on a small host.

**Log files are untrusted input when running as root.** Other users control
everything under their home, so `fun::discover` never follows symlinks while
walking, opens files with `O_NOFOLLOW | O_NONBLOCK` and checks the descriptor
is a regular file (a planted FIFO must not hang the scan, and checking the
descriptor avoids a swap between discovery and open), and caps directory depth
and record size. Only the invoking user's own log roots may be symlinks.

**`--add-retention-expansion` writes only the caller's own settings.** As root,
writing into other homes would mean following paths those users control, which
turns a settings edit into a way to overwrite arbitrary files. It requires a
terminal and an exact typed "yes", plans again after the prompt in case a
running tool saved the file meanwhile, backs up first, and replaces files by
atomic rename. `cleanupPeriodDays` must be at least 1 (Claude Code rejects 0),
so "keep for a long time" is a large number. Codex 0.166 has no session cleanup
setting; only its `[history]` options are touched, and with no TOML library in
the project, unusual forms are reported for manual editing instead of guessed.

**Only human text counts.** Most `user` records in both log formats are tool
results, injected context, or another AI. `fun::parse` filters these so that
praise and abuse reflect what people typed. Claude Code repeats each API
message's usage block on every record of that message, so usage is
deduplicated by message id; Codex logs running totals, so usage is the growth
between records.
