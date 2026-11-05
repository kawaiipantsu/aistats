use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "ai-stats",
    version,
    about = "Collects AI/LLM usage & cost statistics (OpenAI, Anthropic, Ollama, LM Studio, and more) and generates reports.",
    long_about = None,
    arg_required_else_help = true,
    args_conflicts_with_subcommands = true
)]
pub struct Cli {
    #[command(flatten)]
    pub fun: FunArgs,

    #[command(flatten)]
    pub retention: RetentionArgs,

    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Options for the top-level `--add-retention-expansion` mode.
#[derive(Args, Debug)]
pub struct RetentionArgs {
    /// Make your Claude Code and Codex logs last longer by changing their
    /// retention settings. Shows what will change and asks you to type "yes".
    #[arg(long, conflicts_with = "fun_stats")]
    pub add_retention_expansion: bool,
    /// Days of transcripts for Claude Code to keep. Never lowers a higher value.
    #[arg(
        long,
        default_value_t = 3650,
        value_parser = clap::value_parser!(u64).range(1..),
        requires = "add_retention_expansion"
    )]
    pub retention_days: u64,
}

/// Options for the top-level `--fun-stats` mode.
#[derive(Args, Debug)]
pub struct FunArgs {
    /// Generate fun, shareable, PISG-style stats from local Claude Code and
    /// Codex chat logs. Scans every user on the host as root, otherwise only
    /// your own logs. Output holds counts and pseudonyms, never log text.
    #[arg(long)]
    pub fun_stats: bool,
    /// Predefined period: today|yesterday|24h|7d|30d|90d|month|last-month|year|all.
    #[arg(long, default_value = "all", requires = "fun_stats")]
    pub period: String,
    /// Explicit start (overrides --period). RFC3339 or YYYY-MM-DD.
    #[arg(long, requires = "fun_stats")]
    pub from: Option<String>,
    /// Explicit end (overrides --period). RFC3339 or YYYY-MM-DD.
    #[arg(long, requires = "fun_stats")]
    pub to: Option<String>,
    /// Output format: text|markdown|html|json|discord. `html` is a self-contained
    /// page; `discord` is paste-ready text split under Discord's length limit.
    #[arg(long, short = 'f', default_value = "text", requires = "fun_stats")]
    pub format: String,
    /// Write output to this file instead of stdout (e.g. --format html -o stats.html).
    #[arg(long, short = 'o', requires = "fun_stats")]
    pub output: Option<PathBuf>,
    /// Public HTTP(S) URL of the HTML page, for social previews. Exports a
    /// companion PNG beside --output; upload both files to the same directory.
    #[arg(long, requires_all = ["fun_stats", "output"])]
    pub public_url: Option<String>,
    /// Also collect logs over SSH from user@host destinations. Repeat this
    /// option or supply a comma-separated list. Default: local host only.
    #[arg(long = "ssh-host", visible_alias = "ssh-hosts", value_delimiter = ',',
        requires_all = ["fun_stats", "ssh_key"], value_parser = crate::fun::remote::parse_host)]
    pub ssh_hosts: Vec<String>,
    /// Private key for the requested SSH hosts. Uses OpenSSH and verified
    /// known_hosts entries; remote machines need Python 3, not ai-stats.
    #[arg(long, requires_all = ["fun_stats", "ssh_hosts"])]
    pub ssh_key: Option<PathBuf>,
    /// Maximum seconds to collect each SSH host, including connection time.
    #[arg(long, default_value_t = 300, requires = "ssh_hosts",
        value_parser = clap::value_parser!(u64).range(1..))]
    pub ssh_timeout: u64,
    /// Number of entries shown in each ranked list.
    #[arg(long, default_value_t = 5, requires = "fun_stats")]
    pub top: usize,
    /// With --format discord: actually POST the messages to the webhook.
    #[arg(long, requires = "fun_stats")]
    pub send: bool,
    /// Webhook URL to use with --send (overrides config/env).
    #[arg(long, requires = "fun_stats")]
    pub webhook: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Create the config directory and default config.yaml.
    Init,

    /// Inspect or change the configuration file.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// List and test configured providers.
    Providers {
        #[command(subcommand)]
        action: ProvidersAction,
    },

    /// Fetch usage/cost data from provider APIs into the local cache.
    Sync {
        /// Only sync this provider (openai|anthropic|ollama|lmstudio).
        #[arg(long)]
        provider: Option<String>,
        /// Start of the range to fetch. RFC3339 or YYYY-MM-DD.
        /// Defaults to each provider's last synced position, or 7 days ago
        /// on first run.
        #[arg(long)]
        since: Option<String>,
        /// End of the range to fetch. Defaults to now.
        #[arg(long)]
        until: Option<String>,
    },

    /// Import usage data exported from a provider console (CSV/JSON).
    /// Useful for providers without a programmatic usage API, such as
    /// Google Gemini/Vertex AI and xAI.
    Import {
        /// Path to a .csv or .json file.
        file: PathBuf,
        /// Provider name to tag these rows with, if not present in the file.
        #[arg(long)]
        provider: Option<String>,
        /// File format, inferred from the extension when omitted.
        #[arg(long)]
        format: Option<String>,
    },

    /// List models known to local inference servers (Ollama, LM Studio).
    Models {
        #[arg(long)]
        provider: Option<String>,
    },

    /// Generate a usage/cost report from cached data.
    Report {
        /// Restrict to one provider.
        #[arg(long)]
        provider: Option<String>,
        /// Predefined period: today|yesterday|24h|7d|30d|90d|month|last-month|year|all.
        #[arg(long, default_value = "7d")]
        period: String,
        /// Explicit start (overrides --period). RFC3339 or YYYY-MM-DD.
        #[arg(long)]
        from: Option<String>,
        /// Explicit end (overrides --period). RFC3339 or YYYY-MM-DD.
        #[arg(long)]
        to: Option<String>,
        /// Time bucket size for the time-series section: hour|day|month|auto.
        #[arg(long, default_value = "auto")]
        group_by: String,
        /// Output format: text|markdown|json|csv|pdf|discord.
        #[arg(long, short = 'f', default_value = "text")]
        format: String,
        /// Write output to this file instead of stdout.
        #[arg(long, short = 'o')]
        output: Option<PathBuf>,
        /// Number of top models to include in breakdown sections.
        #[arg(long, default_value_t = 10)]
        top: usize,
        /// For --format discord: actually POST the payload to the webhook
        /// instead of just rendering it.
        #[arg(long)]
        send: bool,
        /// Webhook URL to use with --send (overrides config/env).
        #[arg(long)]
        webhook: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum ConfigAction {
    /// Print the resolved config file path.
    Path,
    /// Print the current configuration (secrets masked).
    Show,
    /// Set a configuration value by dotted key path, e.g.
    /// `providers.openai.enabled true`.
    Set { key: String, value: String },
}

#[derive(Subcommand, Debug)]
pub enum ProvidersAction {
    /// List configured providers and whether they're enabled.
    List,
    /// Test connectivity/authentication for one or all providers.
    Test {
        #[arg(long)]
        provider: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser};

    use super::*;

    #[test]
    fn definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn fun_stats_runs_without_a_subcommand() {
        let cli = Cli::try_parse_from(["ai-stats", "--fun-stats", "--format", "discord"]).unwrap();
        assert!(cli.fun.fun_stats);
        assert_eq!(cli.fun.format, "discord");
        assert!(cli.command.is_none());
        assert!(cli.fun.ssh_hosts.is_empty());
        assert!(cli.fun.ssh_key.is_none());
    }

    #[test]
    fn fun_stats_accepts_html_with_an_output_path() {
        for flag in ["--output", "-o"] {
            let cli = Cli::try_parse_from([
                "ai-stats",
                "--fun-stats",
                "--format",
                "html",
                flag,
                "my reports/fun.html",
            ])
            .unwrap();
            assert_eq!(cli.fun.format, "html");
            assert_eq!(cli.fun.output, Some(PathBuf::from("my reports/fun.html")));
        }
    }

    #[test]
    fn ssh_hosts_accept_lists_and_repetition_but_require_explicit_key_and_fun_mode() {
        let cli = Cli::try_parse_from([
            "ai-stats",
            "--fun-stats",
            "--ssh-key",
            "key with spaces",
            "--ssh-hosts",
            "alice@one.example,bob@two.example",
            "--ssh-host",
            "root@three.example",
            "--ssh-timeout",
            "900",
        ])
        .unwrap();
        assert_eq!(
            cli.fun.ssh_hosts,
            ["alice@one.example", "bob@two.example", "root@three.example"]
        );
        assert_eq!(cli.fun.ssh_key, Some(PathBuf::from("key with spaces")));
        assert_eq!(cli.fun.ssh_timeout, 900);
        for args in [
            vec!["ai-stats", "--fun-stats", "--ssh-host", "alice@one.example"],
            vec!["ai-stats", "--fun-stats", "--ssh-key", "key"],
            vec!["ai-stats", "--fun-stats", "--ssh-timeout", "20"],
            vec![
                "ai-stats",
                "--ssh-key",
                "key",
                "--ssh-host",
                "alice@one.example",
            ],
            vec![
                "ai-stats",
                "--fun-stats",
                "--ssh-key",
                "key",
                "--ssh-host",
                "alice@one.example",
                "--ssh-timeout",
                "0",
            ],
        ] {
            assert!(Cli::try_parse_from(args).is_err());
        }
    }

    #[test]
    fn social_export_requires_fun_stats_and_a_destination() {
        let cli = Cli::try_parse_from([
            "ai-stats",
            "--fun-stats",
            "--format",
            "html",
            "-o",
            "index.html",
            "--public-url",
            "https://example.com/",
        ])
        .unwrap();
        assert_eq!(cli.fun.public_url.as_deref(), Some("https://example.com/"));
        assert!(
            Cli::try_parse_from([
                "ai-stats",
                "--fun-stats",
                "--public-url",
                "https://example.com/",
            ])
            .is_err()
        );
    }

    #[test]
    fn fun_options_require_fun_stats() {
        for args in [
            vec!["ai-stats", "--format", "json"],
            vec!["ai-stats", "--send"],
            vec!["ai-stats", "--period", "7d"],
        ] {
            assert!(Cli::try_parse_from(args).is_err());
        }
    }

    #[test]
    fn subcommands_keep_their_own_options() {
        let cli = Cli::try_parse_from(["ai-stats", "report", "--format", "pdf", "--period", "30d"])
            .unwrap();
        assert!(!cli.fun.fun_stats);
        assert!(matches!(
            cli.command,
            Some(Command::Report { format, period, .. }) if format == "pdf" && period == "30d"
        ));
    }

    #[test]
    fn retention_expansion_runs_without_a_subcommand() {
        let cli = Cli::try_parse_from(["ai-stats", "--add-retention-expansion"]).unwrap();
        assert!(cli.retention.add_retention_expansion);
        assert_eq!(cli.retention.retention_days, 3650);
    }

    #[test]
    fn retention_options_are_validated() {
        for args in [
            vec!["ai-stats", "--retention-days", "90"],
            vec![
                "ai-stats",
                "--add-retention-expansion",
                "--retention-days",
                "0",
            ],
            vec!["ai-stats", "--add-retention-expansion", "--fun-stats"],
        ] {
            assert!(Cli::try_parse_from(args).is_err());
        }
    }

    #[test]
    fn fun_stats_cannot_be_combined_with_a_subcommand() {
        assert!(Cli::try_parse_from(["ai-stats", "--fun-stats", "report"]).is_err());
    }
}
