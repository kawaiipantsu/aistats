use std::path::Path;

use anyhow::Result;
use rusqlite::Connection;

use crate::db;
use crate::import::{self, ImportFormat};

pub fn run(
    conn: &mut Connection,
    file: &Path,
    provider: Option<&str>,
    format: Option<&str>,
) -> Result<()> {
    let fmt = match format {
        Some(f) => ImportFormat::parse(f)?,
        None => ImportFormat::from_path(file)?,
    };

    let result = import::import_file(file, fmt, provider)?;
    let n_usage = db::upsert_usage_records(conn, &result.usage)?;
    let n_cost = db::upsert_cost_records(conn, &result.cost)?;

    println!(
        "Imported {} usage row(s) and {} cost row(s) from {}",
        n_usage,
        n_cost,
        file.display()
    );
    Ok(())
}
