//! Renders a Discord webhook payload (embed) summarizing the report.
//! `commands::report_cmd` optionally POSTs this to a configured webhook URL;
//! by default the payload is just printed/saved like any other format so
//! nothing is sent externally unless explicitly requested.

use anyhow::Result;
use serde_json::json;

use crate::aggregate::Report;

use super::fmt_num;

pub fn render_payload(report: &Report, top_models: usize) -> Result<String> {
    let value = build_payload(report, top_models);
    Ok(serde_json::to_string_pretty(&value)?)
}

pub fn build_payload(report: &Report, top_models: usize) -> serde_json::Value {
    let o = &report.overview;

    let mut fields = vec![
        json!({"name": "Requests", "value": fmt_num(o.total_requests), "inline": true}),
        json!({"name": "Input tokens", "value": fmt_num(o.total_input_tokens), "inline": true}),
        json!({"name": "Output tokens", "value": fmt_num(o.total_output_tokens), "inline": true}),
        json!({"name": "Total tokens", "value": fmt_num(o.total_tokens), "inline": true}),
        json!({
            "name": "Total cost",
            "value": format!("${:.4}{}", o.total_cost_usd, if o.cost_partially_estimated { " (partial est.)" } else { "" }),
            "inline": true
        }),
    ];

    if !report.by_model.is_empty() {
        let top: Vec<String> = report
            .by_model
            .iter()
            .take(top_models.min(10))
            .map(|m| {
                format!(
                    "**{}** ({}) — {} tokens{}",
                    m.model,
                    m.provider,
                    fmt_num(m.total_tokens),
                    match m.cost_usd {
                        Some(c) => format!(", ${c:.4}"),
                        None => String::new(),
                    }
                )
            })
            .collect();
        fields.push(json!({"name": "Top models", "value": top.join("\n"), "inline": false}));
    }

    json!({
        "embeds": [{
            "title": "AI Stats — Usage Report",
            "description": format!(
                "**Period:** {} → {} (UTC)\n**Providers:** {}",
                o.since.format("%Y-%m-%d %H:%M"),
                o.until.format("%Y-%m-%d %H:%M"),
                if o.providers.is_empty() { "none".to_string() } else { o.providers.join(", ") }
            ),
            "color": 0x5865F2,
            "fields": fields,
            "footer": {"text": "ai-stats"}
        }]
    })
}
