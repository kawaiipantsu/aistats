//! PDF report rendering using printpdf's built-in Courier (monospace) font,
//! so plain ASCII tables line up without needing to embed a font file.
//! Non-ASCII characters are avoided throughout since the built-in PDF base
//! fonts only cover WinAnsi encoding.

use std::io::{BufWriter, Cursor};

use anyhow::{Result, anyhow};
use printpdf::{BuiltinFont, IndirectFontRef, Line, Mm, PdfDocument, PdfLayerReference, Point};

use crate::aggregate::Report;

use super::fmt_num;

const PAGE_WIDTH_MM: f32 = 210.0;
const PAGE_HEIGHT_MM: f32 = 297.0;
const MARGIN_MM: f32 = 18.0;
const LINE_HEIGHT_MM: f32 = 5.2;

struct Writer {
    doc: printpdf::PdfDocumentReference,
    font: IndirectFontRef,
    font_bold: IndirectFontRef,
    layer: PdfLayerReference,
    y: f32,
}

impl Writer {
    fn new(title: &str) -> Result<Self> {
        let (doc, page1, layer1) =
            PdfDocument::new(title, Mm(PAGE_WIDTH_MM), Mm(PAGE_HEIGHT_MM), "Layer 1");
        let font = doc
            .add_builtin_font(BuiltinFont::Courier)
            .map_err(|e| anyhow!("failed to load builtin font: {e}"))?;
        let font_bold = doc
            .add_builtin_font(BuiltinFont::CourierBold)
            .map_err(|e| anyhow!("failed to load builtin bold font: {e}"))?;
        let layer = doc.get_page(page1).get_layer(layer1);
        Ok(Self {
            doc,
            font,
            font_bold,
            layer,
            y: PAGE_HEIGHT_MM - MARGIN_MM,
        })
    }

    fn ensure_space(&mut self, needed_mm: f32) {
        if self.y - needed_mm < MARGIN_MM {
            let (page, layer) = self
                .doc
                .add_page(Mm(PAGE_WIDTH_MM), Mm(PAGE_HEIGHT_MM), "Layer");
            self.layer = self.doc.get_page(page).get_layer(layer);
            self.y = PAGE_HEIGHT_MM - MARGIN_MM;
        }
    }

    fn line(&mut self, text: &str, size: f32, bold: bool) {
        self.ensure_space(LINE_HEIGHT_MM);
        let font = if bold { &self.font_bold } else { &self.font };
        self.layer
            .use_text(text, size, Mm(MARGIN_MM), Mm(self.y), font);
        self.y -= LINE_HEIGHT_MM;
    }

    fn gap(&mut self, mm: f32) {
        self.y -= mm;
    }

    fn hr(&mut self) {
        self.ensure_space(2.0);
        let y = self.y + 2.0;
        self.layer.add_line(Line {
            points: vec![
                (Point::new(Mm(MARGIN_MM), Mm(y)), false),
                (Point::new(Mm(PAGE_WIDTH_MM - MARGIN_MM), Mm(y)), false),
            ],
            is_closed: false,
        });
        self.y -= 2.0;
    }

    fn finish(self) -> Result<Vec<u8>> {
        let mut writer = BufWriter::new(Cursor::new(Vec::new()));
        self.doc
            .save(&mut writer)
            .map_err(|e| anyhow!("failed to write PDF: {e}"))?;
        let cursor = writer
            .into_inner()
            .map_err(|e| anyhow!("failed to flush PDF buffer: {e}"))?;
        Ok(cursor.into_inner())
    }
}

pub fn render(report: &Report, top_models: usize) -> Result<Vec<u8>> {
    let o = &report.overview;
    let mut w = Writer::new("AI Stats - Usage Report")?;

    w.line("AI STATS - USAGE REPORT", 16.0, true);
    w.gap(2.0);
    w.line(
        &format!(
            "Period: {} to {} (UTC)",
            o.since.format("%Y-%m-%d %H:%M"),
            o.until.format("%Y-%m-%d %H:%M")
        ),
        10.0,
        false,
    );
    w.line(
        &format!(
            "Providers: {}",
            if o.providers.is_empty() {
                "none".to_string()
            } else {
                o.providers.join(", ")
            }
        ),
        10.0,
        false,
    );
    w.gap(3.0);
    w.hr();
    w.gap(2.0);

    w.line("OVERVIEW", 12.0, true);
    w.gap(1.0);
    w.line(
        &format!("Requests:            {}", fmt_num(o.total_requests)),
        10.0,
        false,
    );
    w.line(
        &format!("Input tokens:        {}", fmt_num(o.total_input_tokens)),
        10.0,
        false,
    );
    w.line(
        &format!("Output tokens:       {}", fmt_num(o.total_output_tokens)),
        10.0,
        false,
    );
    w.line(
        &format!("Cached input tokens: {}", fmt_num(o.total_cached_tokens)),
        10.0,
        false,
    );
    w.line(
        &format!("Total tokens:        {}", fmt_num(o.total_tokens)),
        10.0,
        false,
    );
    w.line(
        &format!(
            "Total cost (USD):    ${:.4}{}",
            o.total_cost_usd,
            if o.cost_partially_estimated {
                " (partially estimated)"
            } else {
                ""
            }
        ),
        10.0,
        false,
    );
    w.gap(3.0);

    if !report.by_provider.is_empty() {
        w.hr();
        w.gap(2.0);
        w.line("BY PROVIDER", 12.0, true);
        w.gap(1.0);
        w.line(
            &pad_row(
                &["Provider", "Requests", "Input", "Output", "Total", "Cost"],
                &[14, 10, 12, 12, 12, 14],
            ),
            9.0,
            true,
        );
        for p in &report.by_provider {
            w.line(
                &pad_row(
                    &[
                        &p.provider,
                        &fmt_num(p.requests),
                        &fmt_num(p.input_tokens),
                        &fmt_num(p.output_tokens),
                        &fmt_num(p.total_tokens),
                        &format!(
                            "${:.4}{}",
                            p.cost_usd,
                            if p.cost_estimated { "*" } else { "" }
                        ),
                    ],
                    &[14, 10, 12, 12, 12, 14],
                ),
                9.0,
                false,
            );
        }
        w.gap(3.0);
    }

    if !report.by_model.is_empty() {
        w.hr();
        w.gap(2.0);
        w.line(&format!("TOP {top_models} MODELS BY TOKENS"), 12.0, true);
        w.gap(1.0);
        w.line(
            &pad_row(&["Provider", "Model", "Total", "Cost"], &[12, 26, 12, 14]),
            9.0,
            true,
        );
        for m in report.by_model.iter().take(top_models) {
            let cost = match m.cost_usd {
                Some(c) => format!("${:.4}{}", c, if m.cost_estimated { "*" } else { "" }),
                None => "n/a".to_string(),
            };
            w.line(
                &pad_row(
                    &[
                        &m.provider,
                        &truncate(&m.model, 26),
                        &fmt_num(m.total_tokens),
                        &cost,
                    ],
                    &[12, 26, 12, 14],
                ),
                9.0,
                false,
            );
        }
        w.gap(3.0);
    }

    if !report.time_series.is_empty() {
        w.hr();
        w.gap(2.0);
        w.line(
            &format!("TIME SERIES ({})", report.granularity.as_str()),
            12.0,
            true,
        );
        w.gap(1.0);
        let max_tokens = report
            .time_series
            .iter()
            .map(|r| r.total_tokens)
            .max()
            .unwrap_or(0) as f64;
        w.line(
            &pad_row(
                &["Bucket", "Total Tokens", "Cost", "Chart"],
                &[18, 14, 12, 20],
            ),
            9.0,
            true,
        );
        for r in &report.time_series {
            let bar_width = 20usize;
            let filled = if max_tokens > 0.0 {
                ((r.total_tokens as f64 / max_tokens) * bar_width as f64).round() as usize
            } else {
                0
            };
            let bar = format!(
                "{}{}",
                "#".repeat(filled.min(bar_width)),
                ".".repeat(bar_width - filled.min(bar_width))
            );
            w.line(
                &pad_row(
                    &[
                        &r.label,
                        &fmt_num(r.total_tokens),
                        &format!("${:.2}", r.cost_usd),
                        &bar,
                    ],
                    &[18, 14, 12, 20],
                ),
                9.0,
                false,
            );
        }
    }

    w.gap(4.0);
    w.line(
        &format!(
            "Generated by ai-stats on {}",
            chrono::Utc::now().format("%Y-%m-%d %H:%M UTC")
        ),
        8.0,
        false,
    );

    w.finish()
}

fn pad_row(cols: &[&str], widths: &[usize]) -> String {
    cols.iter()
        .zip(widths)
        .map(|(c, w)| format!("{:<width$}", c, width = w))
        .collect::<Vec<_>>()
        .join(" ")
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}
