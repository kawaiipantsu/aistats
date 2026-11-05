use anyhow::Result;

use crate::aggregate::Report;

pub fn render(report: &Report) -> Result<String> {
    Ok(serde_json::to_string_pretty(report)?)
}
