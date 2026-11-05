# ai-stats: Project-Specific Technology Choices

Language: **Rust** (edition 2024). Key crates and
why each was added:

| Crate | Purpose |
|---|---|
| `clap` (derive) | CLI argument parsing |
| `base64` | Encode vendored WOFF2 fonts into offline HTML; already in the dependency tree via `reqwest` |
| `tokio` | Async runtime for provider HTTP calls |
| `tempfile` | Private temporary SSH log snapshots, automatically removed on return/error |
| `reqwest` (rustls) | HTTP client for OpenAI/Anthropic/Ollama/LM Studio APIs |
| `serde`, `serde_json`, `serde_yaml` | Config (YAML) and report (JSON) (de)serialization. `serde_json` enables `preserve_order` so editing another tool's settings file does not reorder its keys |
| `rusqlite` (bundled) | Local embedded cache database; SQLite is bundled |
| `chrono` | UTC time handling and bucket flooring for hourly/daily/monthly rollups |
| `csv` | CSV import and export |
| `comfy-table` | Aligned tables for the text report format |
| `printpdf` | PDF generation using the built-in base-14 fonts (Courier), so no external font file needs to be sourced/licensed per [the asset registry](ASSETS.md) |
| `anyhow` / `thiserror` | Error handling |
| `async-trait` | Object-safe async `UsageProvider` trait so providers can be stored as `Vec<Box<dyn UsageProvider>>` |
| `libc` (Unix only) | `geteuid` to decide what `--fun-stats` may scan, and `O_NOFOLLOW`/`O_NONBLOCK`, whose values differ per platform, for opening log files safely. Already in the tree via other crates |

Opt-in SSH collection uses the system OpenSSH client and Python 3 on remote
Unix-like hosts. Tokio process/I/O/filesystem support streams the bundled,
read-only Python helper's framed output with a per-host timeout. No remote
ai-stats installation is required; local-only runs need neither SSH nor Python.
Run helper tests with `python3 -B -m unittest discover -s src/fun -p remote_test.py`.

Build with current stable Rust and a C compiler. Native CI targets Linux AMD64
and ARM64. HTTPS uses Rustls, so system OpenSSL development headers are not
needed. The checked-in Cargo.lock pins the application dependency versions.
