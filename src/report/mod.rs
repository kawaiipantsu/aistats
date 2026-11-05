//! Output renderers. Every renderer consumes the same [`crate::aggregate::Report`]
//! so all formats always agree on the underlying numbers.

pub mod csv_out;
pub mod discord;
pub mod fun;
pub mod json_out;
pub mod markdown;
pub mod pdf;
pub mod txt;

use anyhow::{Result, bail};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Text,
    Markdown,
    Html,
    Json,
    Csv,
    Pdf,
    Discord,
}

impl OutputFormat {
    pub fn parse(s: &str) -> Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "text" | "txt" => Ok(OutputFormat::Text),
            "markdown" | "md" => Ok(OutputFormat::Markdown),
            "html" => Ok(OutputFormat::Html),
            "json" => Ok(OutputFormat::Json),
            "csv" => Ok(OutputFormat::Csv),
            "pdf" => Ok(OutputFormat::Pdf),
            "discord" => Ok(OutputFormat::Discord),
            other => bail!(
                "unknown output format '{other}' (expected text|markdown|html|json|csv|pdf|discord)"
            ),
        }
    }
}

/// A rendered report body, either text (for stdout/file writes as UTF-8) or
/// raw bytes (PDF).
pub enum Rendered {
    Text(String),
    Bytes(Vec<u8>),
}

/// Renders a simple filled/empty block bar for lightweight text charts,
/// e.g. "████████░░░░░░░░" for a value at 50% of `max`.
pub fn ascii_bar(value: f64, max: f64, width: usize) -> String {
    if max <= 0.0 || width == 0 {
        return String::new();
    }
    let filled = ((value / max) * width as f64)
        .round()
        .clamp(0.0, width as f64) as usize;
    let mut s = String::with_capacity(width);
    for _ in 0..filled {
        s.push('█');
    }
    for _ in filled..width {
        s.push('░');
    }
    s
}

/// Formats an integer with thousands separators, e.g. `1234567` -> `1,234,567`.
pub fn fmt_num(n: i64) -> String {
    let s = n.unsigned_abs().to_string();
    let mut grouped = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    let mut grouped: String = grouped.chars().rev().collect();
    if n < 0 {
        grouped.insert(0, '-');
    }
    grouped
}

pub fn render(
    report: &crate::aggregate::Report,
    format: OutputFormat,
    top_models: usize,
) -> Result<Rendered> {
    Ok(match format {
        OutputFormat::Text => Rendered::Text(txt::render(report, top_models)),
        OutputFormat::Markdown => Rendered::Text(markdown::render(report, top_models)),
        OutputFormat::Html => bail!("HTML output requires --fun-stats --format html"),
        OutputFormat::Json => Rendered::Text(json_out::render(report)?),
        OutputFormat::Csv => Rendered::Text(csv_out::render(report)?),
        OutputFormat::Pdf => Rendered::Bytes(pdf::render(report, top_models)?),
        OutputFormat::Discord => Rendered::Text(discord::render_payload(report, top_models)?),
    })
}
