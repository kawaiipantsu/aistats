mod aggregate;
mod cli;
mod commands;
mod config;
mod db;
mod fun;
mod import;
mod models;
mod paths;
mod pricing;
mod providers;
mod report;
mod time_util;

use anyhow::Result;
use clap::Parser;

use cli::{Cli, Command, ConfigAction, ProvidersAction};
use config::Config;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    if let Err(e) = run(cli).await {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
    Ok(())
}

async fn run(cli: Cli) -> Result<()> {
    let config_path = paths::config_file()?;

    if cli.retention.add_retention_expansion {
        return commands::retention_cmd::run(cli.retention.retention_days);
    }

    let Some(command) = cli.command else {
        let cfg = Config::load_or_default(&config_path)?;
        return commands::fun_cmd::run(&cfg, &cli.fun).await;
    };

    match command {
        Command::Init => commands::init::run(),

        Command::Config { action } => match action {
            ConfigAction::Path => commands::config_cmd::path(&config_path),
            ConfigAction::Show => commands::config_cmd::show(&config_path),
            ConfigAction::Set { key, value } => {
                commands::config_cmd::set(&config_path, &key, &value)
            }
        },

        Command::Providers { action } => {
            let cfg = Config::load_or_default(&config_path)?;
            match action {
                ProvidersAction::List => {
                    commands::providers_cmd::list(&cfg);
                    Ok(())
                }
                ProvidersAction::Test { provider } => {
                    commands::providers_cmd::test(&cfg, provider.as_deref()).await
                }
            }
        }

        Command::Sync {
            provider,
            since,
            until,
        } => {
            let cfg = Config::load_or_default(&config_path)?;
            let mut conn = db::open(&paths::database_file()?)?;
            commands::sync_cmd::run(
                &cfg,
                &mut conn,
                provider.as_deref(),
                since.as_deref(),
                until.as_deref(),
            )
            .await
        }

        Command::Import {
            file,
            provider,
            format,
        } => {
            let mut conn = db::open(&paths::database_file()?)?;
            commands::import_cmd::run(&mut conn, &file, provider.as_deref(), format.as_deref())
        }

        Command::Models { provider } => {
            let cfg = Config::load_or_default(&config_path)?;
            commands::models_cmd::run(&cfg, provider.as_deref()).await
        }

        Command::Report {
            provider,
            period,
            from,
            to,
            group_by,
            format,
            output,
            top,
            send,
            webhook,
        } => {
            let cfg = Config::load_or_default(&config_path)?;
            let conn = db::open(&paths::database_file()?)?;
            commands::report_cmd::run(
                &cfg,
                &conn,
                provider.as_deref(),
                &period,
                from.as_deref(),
                to.as_deref(),
                &group_by,
                &format,
                output.as_deref(),
                top,
                send,
                webhook.as_deref(),
            )
            .await
        }
    }
}
