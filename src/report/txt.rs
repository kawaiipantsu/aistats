use comfy_table::{Cell, ContentArrangement, Table, presets::UTF8_FULL};

use crate::aggregate::Report;

use super::{ascii_bar, fmt_num};

pub fn render(report: &Report, top_models: usize) -> String {
    let o = &report.overview;
    let mut out = String::new();

    out.push_str("AI STATS — USAGE REPORT\n");
    out.push_str(&"=".repeat(72));
    out.push('\n');
    out.push_str(&format!(
        "Period:    {} → {} (UTC)\n",
        o.since.format("%Y-%m-%d %H:%M"),
        o.until.format("%Y-%m-%d %H:%M")
    ));
    out.push_str(&format!(
        "Providers: {}\n",
        if o.providers.is_empty() {
            "none".to_string()
        } else {
            o.providers.join(", ")
        }
    ));
    out.push('\n');

    out.push_str("OVERVIEW\n");
    out.push_str(&"-".repeat(72));
    out.push('\n');
    out.push_str(&format!(
        "Requests:            {}\n",
        fmt_num(o.total_requests)
    ));
    out.push_str(&format!(
        "Input tokens:        {}\n",
        fmt_num(o.total_input_tokens)
    ));
    out.push_str(&format!(
        "Output tokens:       {}\n",
        fmt_num(o.total_output_tokens)
    ));
    out.push_str(&format!(
        "Cached input tokens: {}\n",
        fmt_num(o.total_cached_tokens)
    ));
    out.push_str(&format!(
        "Total tokens:        {}\n",
        fmt_num(o.total_tokens)
    ));
    out.push_str(&format!(
        "Total cost (USD):    ${:.4}{}\n",
        o.total_cost_usd,
        if o.cost_partially_estimated {
            " (partially estimated, see notes)"
        } else {
            ""
        }
    ));
    out.push('\n');

    if !report.by_provider.is_empty() {
        out.push_str("BY PROVIDER\n");
        out.push_str(&"-".repeat(72));
        out.push('\n');
        let mut table = Table::new();
        table
            .load_style(UTF8_FULL)
            .set_content_arrangement(ContentArrangement::Dynamic)
            .set_header(vec![
                "Provider",
                "Requests",
                "Input",
                "Output",
                "Total",
                "Cost (USD)",
            ]);
        for p in &report.by_provider {
            table.add_row(vec![
                Cell::new(&p.provider),
                Cell::new(fmt_num(p.requests)),
                Cell::new(fmt_num(p.input_tokens)),
                Cell::new(fmt_num(p.output_tokens)),
                Cell::new(fmt_num(p.total_tokens)),
                Cell::new(format!(
                    "${:.4}{}",
                    p.cost_usd,
                    if p.cost_estimated { " (est.)" } else { "" }
                )),
            ]);
        }
        out.push_str(&table.to_string());
        out.push_str("\n\n");
    }

    if !report.by_model.is_empty() {
        out.push_str(&format!("TOP {top_models} MODELS BY TOKENS\n"));
        out.push_str(&"-".repeat(72));
        out.push('\n');
        let mut table = Table::new();
        table
            .load_style(UTF8_FULL)
            .set_content_arrangement(ContentArrangement::Dynamic)
            .set_header(vec![
                "Provider",
                "Model",
                "Requests",
                "Input",
                "Output",
                "Total",
                "Cost (USD)",
            ]);
        for m in report.by_model.iter().take(top_models) {
            table.add_row(vec![
                Cell::new(&m.provider),
                Cell::new(&m.model),
                Cell::new(fmt_num(m.requests)),
                Cell::new(fmt_num(m.input_tokens)),
                Cell::new(fmt_num(m.output_tokens)),
                Cell::new(fmt_num(m.total_tokens)),
                Cell::new(match m.cost_usd {
                    Some(c) => {
                        format!("${:.4}{}", c, if m.cost_estimated { " (est.)" } else { "" })
                    }
                    None => "n/a".to_string(),
                }),
            ]);
        }
        out.push_str(&table.to_string());
        out.push_str("\n\n");
    }

    if !report.time_series.is_empty() {
        out.push_str(&format!("TIME SERIES ({})\n", report.granularity.as_str()));
        out.push_str(&"-".repeat(72));
        out.push('\n');
        let max_tokens = report
            .time_series
            .iter()
            .map(|r| r.total_tokens)
            .max()
            .unwrap_or(0) as f64;
        let mut table = Table::new();
        table
            .load_style(UTF8_FULL)
            .set_content_arrangement(ContentArrangement::Dynamic)
            .set_header(vec!["Bucket", "Total Tokens", "Cost (USD)", ""]);
        for r in &report.time_series {
            table.add_row(vec![
                Cell::new(&r.label),
                Cell::new(fmt_num(r.total_tokens)),
                Cell::new(format!("${:.4}", r.cost_usd)),
                Cell::new(ascii_bar(r.total_tokens as f64, max_tokens, 24)),
            ]);
        }
        out.push_str(&table.to_string());
        out.push('\n');
    }

    out
}
