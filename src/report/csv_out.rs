//! CSV export of the bucketed time series — the shape most useful for
//! spreadsheets and downstream charting of hourly/daily/monthly consumption.

use anyhow::Result;

use crate::aggregate::Report;

pub fn render(report: &Report) -> Result<String> {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    wtr.write_record([
        "bucket",
        "requests",
        "input_tokens",
        "output_tokens",
        "total_tokens",
        "cost_usd",
    ])?;
    for row in &report.time_series {
        wtr.write_record(&[
            row.label.clone(),
            row.requests.to_string(),
            row.input_tokens.to_string(),
            row.output_tokens.to_string(),
            row.total_tokens.to_string(),
            format!("{:.6}", row.cost_usd),
        ])?;
    }
    let bytes = wtr.into_inner()?;
    Ok(String::from_utf8(bytes)?)
}
