# Usage guide

See the [README](../README.md#install) for installation and your first report.

## ⚙️ Configuration

Everything the CLI owns lives under `~/.config/ai-stats/`:

```text
~/.config/ai-stats/
├── config.yaml     # providers, keys, pricing overrides, Discord webhook
├── cache/
│   └── usage.db    # local SQLite cache of normalized usage/cost data
├── logs/
└── reports/        # a convenient default place to write report output
```

`config.yaml` is created by `ai-stats init` and is permission-restricted to the
owning user (`0600`) since it can hold API keys. You can instead keep secrets
out of the file entirely and set them as environment variables, which always
take priority:

| Config key | Environment variable |
|---|---|
| `providers.openai.admin_api_key` | `OPENAI_ADMIN_API_KEY` |
| `providers.anthropic.admin_api_key` | `ANTHROPIC_ADMIN_API_KEY` |
| `discord.webhook_url` | `AI_STATS_DISCORD_WEBHOOK_URL` |

Change any setting with `ai-stats config set <dotted.key> <value>`, e.g.:

```bash
ai-stats config set providers.anthropic.enabled true
ai-stats config set providers.ollama.base_url http://192.168.1.20:11434
ai-stats config set pricing.overrides.my-custom-model.input_per_million 5.0
```

### Supported providers

| Provider | How usage is collected | Requires |
|---|---|---|
| **OpenAI** | [Usage & Costs API](https://platform.openai.com/docs/api-reference/usage) | An **organization admin key** (`sk-admin-...`) — a regular project API key cannot read this data. |
| **Anthropic** | [Usage & Cost Admin API](https://platform.claude.com/docs/en/api/usage-cost-api) | An **Admin API key** (`sk-ant-admin01-...`). |
| **Ollama** | Local `/api/tags` + `/api/ps` | A running local Ollama server. No token-usage history (Ollama doesn't record it) — this gives you the model catalog and what's currently loaded. |
| **LM Studio** | Local `/v1/models` | A running local LM Studio server. Same limitation as Ollama: model inventory only, no historical token usage. |
| **Google Gemini / Vertex AI, xAI, others** | `ai-stats import` | Import CSV/JSON exports from the provider's console — see below. |

### Importing usage from other providers

```bash
ai-stats import google-usage.csv --provider google
```

Expected columns (extra columns are ignored): `provider, model, date, input_tokens, output_tokens, requests, cost_usd, cached_input_tokens`. `provider` may be omitted from the file if `--provider` is passed instead. `date` accepts `YYYY-MM-DD` or a full RFC3339 timestamp. JSON files use the same fields as an array of objects.

## 📊 Reports

```bash
ai-stats report \
  --provider openai \
  --period month \
  --group-by day \
  --format pdf \
  --output monthly-report.pdf
```

| Flag | Purpose |
|---|---|
| `--period` | `today, yesterday, 24h, 7d, 30d, 90d, month, last-month, year, all` |
| `--from` / `--to` | Explicit range (`YYYY-MM-DD` or RFC3339), overrides `--period` |
| `--group-by` | Time-series bucket size: `hour, day, month, auto` |
| `--format` | `text, markdown, json, csv, pdf, discord` |
| `--provider` | Restrict to one provider |
| `--top` | Number of models shown in the top-models breakdown |
| `-o, --output` | Write to a file instead of stdout (required for `pdf`) |

Send a report straight to a Discord channel:

```bash
ai-stats report --period 7d --format discord --send \
  --webhook https://discord.com/api/webhooks/...
```

Without `--send`, `--format discord` just renders the webhook JSON payload like any other format — nothing is posted unless you explicitly ask for it.

## 🎭 Fun Stats

Remember [PISG](https://pisg.github.io/)? `--fun-stats` does the same for your AI chats. It reads local Claude Code and Codex session logs and produces awards, activity charts, and everything else you never wanted to know about how you talk to a language model.

```bash
# Everyone on the host (needs root), or just your own logs otherwise.
sudo ai-stats --fun-stats

# Paste-ready Discord messages, split to fit Discord's 2,000 character limit.
ai-stats --fun-stats --format discord --period 30d

# Export a self-contained HTML page to a file of your choice.
ai-stats --fun-stats --format html --period 30d --output ./fun-stats.html

# For sharing a hosted link: adds OG/X cards and exports a companion preview PNG.
ai-stats --fun-stats --format html --output ./index.html --public-url https://stats.example.com/

# Include other machines, in addition to this host (repeat --ssh-host or use commas).
ai-stats --fun-stats --ssh-key ~/.ssh/id_ed25519 \
  --ssh-host workstation.example,server.example --format html -o index.html

# Or post them straight to a webhook.
ai-stats --fun-stats --format discord --send --webhook https://discord.com/api/webhooks/...
```

```text
🎭 AI Stats: Fun Stats
========================================================================
📅 2026-09-01 → 2026-09-30 · 👥 3 users · 🤖 Claude + Codex
💬 4,120 prompts · 🗂️ 310 sessions · 🔧 51,200 tool calls
🔥 1.2B tokens burned (98.4M fresh, 1.1B from cache): the whole Harry Potter series, 840 times over

🏆 Hall of Fame
------------------------------------------------------------------------
🦉 Night Owl — 🦊 Sleepy Fox: 41% of prompts between midnight and 5am (sleep is a deprecated feature)
🤬 Swear Jar Champion — 🐐 Salty Goat: 212 swears, owes the jar $53.00 (the logs remember)
🎩 The Diplomat — 🦙 Polite Llama: 640 pleases and thank-yous (will be spared in the uprising)
🛑 Ctrl-C Cowboy — 🐐 Salty Goat: cut the AI off 87 times (rude, but fast)
```

The report covers awards, activity by hour, what the humans said (swearing, praise, AI abuse, hate speech, manners, smileys, emoji, excessive punctuation, longest words, begging, threats, typos), what the AI did (flattery, apologies, buzzwords, em dashes, failed tools, "should work now" claims that weren't), a Claude vs Codex matchup, risky commands, hot topics, naming crimes, and short quotes.

| Flag | Purpose |
|---|---|
| `--format`, `-f` | `text`, `markdown`, `html` (self-contained page), `json`, `discord` (paste-ready messages) |
| `--period` | Same values as `report`. Defaults to `all` |
| `--from` / `--to` | Explicit range (`YYYY-MM-DD` or RFC3339), overrides `--period` |
| `--top` | Entries in each ranked list (default 5) |
| `-o, --output` | Write to a file instead of stdout |
| `--public-url` | Public HTTP(S) page URL for HTML social previews; requires `--format html` and `--output` |
| `--send`, `--webhook` | Post to Discord. Requires `--format discord` |

**HTML export.** Open the resulting `.html` file directly in a browser or share it
as a single file. Styles, SVG artwork, and three web fonts are embedded, with no
scripts, downloads, or internet connection needed. Bricolage Grotesque titles,
Space Grotesk section headings, and DM Sans body text come from Google Fonts;
their OFL licenses travel with the export under “Type & credits.” Wide, full-width
stat sections reduce desktop wrapping, while phones and printouts get their own
layouts. The robot and background have gentle CSS animations, a “Pause motion”
control, and support for reduced-motion preferences. The date span includes the
number of calendar days, counting both endpoints. Use `--output` / `-o`
with a relative or absolute file path (the parent directory must exist); without
it, HTML goes to stdout for piping or redirection. `--period`, `--from`, `--to`,
and `--top` work as usual. An empty period still exports a page explaining that
no sessions were found.

**Social previews.** Every HTML export includes a fun title and a description
based on the report totals. Add `--public-url https://your-host/path/` (or the
full URL of the HTML file) for canonical, Open Graph, and X/Twitter large-image
tags. This writes `ai-stats-social-v1.png` beside the HTML: upload both to the
same public directory. The preview is a bundled 1200 × 630 PNG, with no runtime
image tools needed. The HTML itself remains self-contained. OG images need
[public HTTP(S) URLs](https://ogp.me/#data-types), so the SVG artwork lives within
the page and a PNG is used for social previews. Without `--public-url`, no public
address is guessed and the export uses a text card. Social services may cache
older previews after a page changes.

**Daily refresh.** `contrib/cron/ai-stats-fun-stats` contains a root crontab
example that runs at 04:15 daily in the server's timezone. Install
`contrib/cron/ai-stats-refresh-html` into `/usr/local/sbin/` as shown in the
example, then add the job with `crontab -e`, adjusting its output path and
public URL. The job uses `flock` to avoid overlap, records output in
`~/.config/ai-stats/logs/fun-stats-refresh.log`, and generates files in a private
staging directory before atomically replacing the published HTML. A failed
export leaves the previous page available. Retention expansion is a one-time
interactive settings change; it is not part of the cron job.

To include remote machines in a scheduled export, append `--ssh-key
/absolute/path/to/key --ssh-host user@host.example` after the wrapper's output
path and public URL. Keep those deployment-specific arguments in your private
crontab, not in repository examples. Use an unencrypted deployment key or an
available SSH agent: unattended runs never prompt for passwords/passphrases.

**Multiple machines over SSH.** Collection is local-only unless you pass
`--ssh-host` (alias `--ssh-hosts`) together with `--ssh-key`. Hosts can be
comma-separated or repeated. Local logs are always included. The remote host
needs a Unix-like OS and Python 3; it does not need ai-stats installed. The
collector streams a read-only helper over OpenSSH and downloads only Claude
Code/Codex session and history logs into private temporary directories, removed
after analysis or an ordinary error. Tool credentials and settings are not
downloaded or modified. Abrupt termination such as SIGKILL can leave temporary
directories behind in the system temp directory.

The same root/all-users versus non-root/own-account rule applies independently
on every host. Accounts with the same name on different hosts get distinct
pseudonyms. Reports show the total host count without exposing host addresses,
usernames or key paths. Repeated identical destinations are collected once;
different aliases for the same machine and logs copied across machines are
treated as separate sources, so list each machine only once.

Host keys must already be verified in OpenSSH's `known_hosts`. Connect normally
and verify a new host's fingerprint before scheduling it; verification is never
disabled. SSH config can supply ports or jump hosts. Each host has a five-minute
collection timeout, adjustable with `--ssh-timeout SECONDS`, and a 32 GiB /
100,000-file transfer limit. A failed host aborts the export, preserving the
previous page when using the publishing wrapper. Date filters apply to the
combined report; the collector still transfers the full available logs.

**The Paper Trail.** Fun stats also estimate the A4 pages, paper-stack height,
paper weight, end-to-end length, novels, and reading time represented by the
full text of counted prompts (including pasted material). These comparisons
assume 3,000 characters per A4 page, 6 characters per word, 90,000 words per
novel, and 250 words per minute. Physical paper assumes one continuous,
single-sided printout, rounding up to whole sheets, at 80 g/m² and 0.1 mm thick.
Tree equivalents normalize that mass using [PrintReleaf's standard paper
conversion](https://resources.printreleaf.com/PrintReleaf-Developer-Guide-MPS-v1.6.1.pdf)
(8,333 letter sheets at 74 g/m², about 37.2 kg per standard tree). This is a
hypothetical printing comparison, not measured trees saved or AI carbon savings.

**Who gets scanned.** As root, every account in `/etc/passwd` plus any directory under `/home`, including service accounts with homes elsewhere. Otherwise only your own logs, however readable other homes are. It reads `~/.claude/projects`, `~/.config/claude/projects`, `~/.codex/sessions`, `~/.codex/archived_sessions`, and both tools' `history.jsonl`.

**Safe to share by design.** The report is built from counters, not text:

- Every label comes from a table compiled into the binary. Text from a log is matched against those tables but never copied into the report, so a secret in a prompt has no way out. The same goes for model, tool, and command names, which are mapped to fixed labels.
- Account names become stable pseudonyms such as `🦦 Sleepy Otter`, derived with a random salt kept in `cache/fun-stats.salt`. Hostnames and paths are never included.
- Project and domain names are roasted, not printed. A verdict keeps at most the first letter, the TLD, and sometimes the length (`f••••-final`, `••••.xyz`), and names under six letters show no letters at all. Names built entirely from generic words, like `tmp` or `test2`, are printed because they say nothing. These results are host-wide and never tied to a pseudonym.
- Quotes are rebuilt word by word from a built-in vocabulary with no names, numbers, or credential, family, health, or money words. A sentence containing any other word cannot be quoted, and quotes are not attributed to anyone.
- Swear words are masked (`f**k`). Slurs are never printed at all: they are counted by class (`ableist`, `racist`) as a host-wide total, never tied to a pseudonym.

Pseudonyms hide names, not behaviour: on a small host, people may recognize each other by their habits. Check a report before posting it somewhere public.

**Limitations.** Detection is English word matching, so it misses sarcasm and other languages. Slur matching favours precision, so words with a common innocent meaning are left out. Claude Code deletes transcripts after 30 days by default, so older sessions contribute only their prompts, from `history.jsonl`. Run `ai-stats --add-retention-expansion` to keep more.

## 🗄️ Keeping Logs Longer

Claude Code deletes transcripts older than 30 days by default, which also limits what `--fun-stats` can see.

```bash
ai-stats --add-retention-expansion                      # keep Claude Code transcripts for 10 years
ai-stats --add-retention-expansion --retention-days 365
```

It shows exactly what will change, then waits for you to type `yes` and press Enter. Anything else cancels, and it refuses to run without a terminal, so a piped `yes` can't confirm it.

| Tool | What changes |
|---|---|
| **Claude Code** | Sets `cleanupPeriodDays` in `settings.json` (or under `CLAUDE_CONFIG_DIR`). A higher existing value is never lowered. |
| **Codex** | Has no session cleanup setting, so usually nothing. If `config.toml` (or under `CODEX_HOME`) limits `history.jsonl`, it sets `persistence` to `save-all` and comments out `max_bytes`. |

Each changed file is backed up beside itself as `*.ai-stats-backup-<timestamp>`. Only your own settings change, including when run as root: run it as each user who wants longer logs. A `cleanupPeriodDays` in project or managed settings outranks your user settings, and logs that were already deleted cannot be restored.

## 🧭 Command Reference

```text
ai-stats init                          Create the config dir and default config.yaml
ai-stats config {show,path,set}        Inspect or change configuration
ai-stats providers {list,test}         List/test provider connectivity
ai-stats sync [--provider] [--since]   Fetch usage/cost into the local cache
ai-stats import <file> [--provider]    Import usage exported from a console
ai-stats models [--provider]           List local Ollama/LM Studio model inventory
ai-stats report [...]                  Generate a report (see above)
ai-stats --fun-stats [...]             Fun, shareable stats from Claude/Codex chat logs
ai-stats --add-retention-expansion     Make Claude/Codex logs last longer (asks first)
```
