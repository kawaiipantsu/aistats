//! Turns raw usage/cost rows from the database into report-ready rollups:
//! an overview, per-provider and per-model breakdowns, and a bucketed time
//! series (hourly/daily/monthly) suitable for every output renderer.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde::Serialize;

use crate::config::PricingConfig;
use crate::db;
use crate::models::Granularity;
use crate::pricing;
use crate::time_util::{floor_to_bucket, format_bucket_label};

#[derive(Debug, Clone, Serialize)]
pub struct Overview {
    pub since: DateTime<Utc>,
    pub until: DateTime<Utc>,
    pub total_requests: i64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_cached_tokens: i64,
    pub total_tokens: i64,
    pub total_cost_usd: f64,
    /// True when at least part of the total cost came from the local
    /// pricing-table estimate rather than a provider-reported cost API.
    pub cost_partially_estimated: bool,
    pub providers: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderBreakdown {
    pub provider: String,
    pub requests: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub cost_usd: f64,
    pub cost_estimated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelBreakdown {
    pub provider: String,
    pub model: String,
    pub requests: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_input_tokens: i64,
    pub total_tokens: i64,
    /// `None` when the provider bills at an org level without a per-model
    /// split we can attach (known limitation of some cost APIs).
    pub cost_usd: Option<f64>,
    pub cost_estimated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct TimeBucketRow {
    pub bucket_start: DateTime<Utc>,
    pub label: String,
    pub requests: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub cost_usd: f64,
}

#[derive(Serialize)]
pub struct Report {
    pub overview: Overview,
    pub by_provider: Vec<ProviderBreakdown>,
    pub by_model: Vec<ModelBreakdown>,
    pub time_series: Vec<TimeBucketRow>,
    pub granularity: Granularity,
}

pub fn build_report(
    conn: &Connection,
    since: DateTime<Utc>,
    until: DateTime<Utc>,
    provider_filter: Option<&str>,
    granularity: Granularity,
    pricing_cfg: &PricingConfig,
) -> anyhow::Result<Report> {
    let usage = db::query_usage(conn, since, until, provider_filter)?;
    let cost = db::query_cost(conn, since, until, provider_filter)?;

    let providers_with_cost: HashSet<String> = cost.iter().map(|c| c.provider.clone()).collect();

    let mut cost_by_provider: HashMap<String, f64> = HashMap::new();
    let mut cost_by_provider_model: HashMap<(String, String), f64> = HashMap::new();
    for c in &cost {
        *cost_by_provider.entry(c.provider.clone()).or_insert(0.0) += c.amount_usd;
        if let Some(model) = &c.model {
            *cost_by_provider_model
                .entry((c.provider.clone(), model.clone()))
                .or_insert(0.0) += c.amount_usd;
        }
    }

    // Per-model aggregation (also rolls up into per-provider and overview).
    struct Acc {
        provider: String,
        model: String,
        requests: i64,
        input_tokens: i64,
        output_tokens: i64,
        cached_input_tokens: i64,
    }
    let mut by_model_acc: HashMap<(String, String), Acc> = HashMap::new();
    let mut time_acc: HashMap<DateTime<Utc>, TimeBucketRow> = HashMap::new();

    for r in &usage {
        let key = (r.provider.clone(), r.model.clone());
        let entry = by_model_acc.entry(key).or_insert_with(|| Acc {
            provider: r.provider.clone(),
            model: r.model.clone(),
            requests: 0,
            input_tokens: 0,
            output_tokens: 0,
            cached_input_tokens: 0,
        });
        entry.requests += r.requests;
        entry.input_tokens += r.input_tokens;
        entry.output_tokens += r.output_tokens;
        entry.cached_input_tokens += r.cached_input_tokens;

        let bucket = floor_to_bucket(r.bucket_start, granularity);
        let row = time_acc.entry(bucket).or_insert_with(|| TimeBucketRow {
            bucket_start: bucket,
            label: format_bucket_label(bucket, granularity),
            requests: 0,
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
            cost_usd: 0.0,
        });
        row.requests += r.requests;
        row.input_tokens += r.input_tokens;
        row.output_tokens += r.output_tokens;
        row.total_tokens += r.input_tokens + r.output_tokens;
    }

    // Distribute cost into time buckets proportionally to token volume when
    // the provider only reports cost at a coarser (daily) granularity than
    // the requested bucket; for same-or-coarser granularity this still lands
    // in the right bucket since cost buckets are always whole days.
    for c in &cost {
        let bucket = floor_to_bucket(c.bucket_start, granularity);
        let row = time_acc.entry(bucket).or_insert_with(|| TimeBucketRow {
            bucket_start: bucket,
            label: format_bucket_label(bucket, granularity),
            requests: 0,
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
            cost_usd: 0.0,
        });
        row.cost_usd += c.amount_usd;
    }

    let mut by_model: Vec<ModelBreakdown> = Vec::new();
    let mut cost_by_provider_estimated: HashMap<String, f64> = HashMap::new();
    let mut any_estimated = false;

    for acc in by_model_acc.into_values() {
        let has_provider_cost = providers_with_cost.contains(&acc.provider);
        let (cost_usd, estimated) = if has_provider_cost {
            match cost_by_provider_model.get(&(acc.provider.clone(), acc.model.clone())) {
                Some(v) => (Some(*v), false),
                None => (None, false),
            }
        } else {
            let est = pricing::estimate_cost_usd(
                &acc.model,
                acc.input_tokens,
                acc.output_tokens,
                acc.cached_input_tokens,
                pricing_cfg,
            );
            if let Some(v) = est {
                *cost_by_provider_estimated
                    .entry(acc.provider.clone())
                    .or_insert(0.0) += v;
                any_estimated = true;
            }
            (est, est.is_some())
        };

        by_model.push(ModelBreakdown {
            provider: acc.provider,
            model: acc.model,
            requests: acc.requests,
            input_tokens: acc.input_tokens,
            output_tokens: acc.output_tokens,
            cached_input_tokens: acc.cached_input_tokens,
            total_tokens: acc.input_tokens + acc.output_tokens,
            cost_usd,
            cost_estimated: estimated,
        });
    }
    by_model.sort_by_key(|row| std::cmp::Reverse(row.total_tokens));

    // Per-provider rollup.
    let mut provider_totals: HashMap<String, (i64, i64, i64)> = HashMap::new();
    for m in &by_model {
        let e = provider_totals
            .entry(m.provider.clone())
            .or_insert((0, 0, 0));
        e.0 += m.requests;
        e.1 += m.input_tokens;
        e.2 += m.output_tokens;
    }
    let mut by_provider: Vec<ProviderBreakdown> = provider_totals
        .into_iter()
        .map(|(provider, (requests, input_tokens, output_tokens))| {
            let has_cost = providers_with_cost.contains(&provider);
            let cost_usd = if has_cost {
                *cost_by_provider.get(&provider).unwrap_or(&0.0)
            } else {
                *cost_by_provider_estimated.get(&provider).unwrap_or(&0.0)
            };
            ProviderBreakdown {
                provider,
                requests,
                input_tokens,
                output_tokens,
                total_tokens: input_tokens + output_tokens,
                cost_usd,
                cost_estimated: !has_cost,
            }
        })
        .collect();
    by_provider.sort_by_key(|row| std::cmp::Reverse(row.total_tokens));

    let mut time_series: Vec<TimeBucketRow> = time_acc.into_values().collect();
    time_series.sort_by_key(|r| r.bucket_start);
    for r in &mut time_series {
        r.total_tokens = r.input_tokens + r.output_tokens;
    }

    let total_requests: i64 = by_provider.iter().map(|p| p.requests).sum();
    let total_input_tokens: i64 = by_provider.iter().map(|p| p.input_tokens).sum();
    let total_output_tokens: i64 = by_provider.iter().map(|p| p.output_tokens).sum();
    let total_cached_tokens: i64 = by_model.iter().map(|m| m.cached_input_tokens).sum();
    let total_cost_usd: f64 = by_provider.iter().map(|p| p.cost_usd).sum();
    let providers: Vec<String> = by_provider.iter().map(|p| p.provider.clone()).collect();

    let overview = Overview {
        since,
        until,
        total_requests,
        total_input_tokens,
        total_output_tokens,
        total_cached_tokens,
        total_tokens: total_input_tokens + total_output_tokens,
        total_cost_usd,
        cost_partially_estimated: any_estimated,
        providers,
    };

    Ok(Report {
        overview,
        by_provider,
        by_model,
        time_series,
        granularity,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::UsageRecord;
    use chrono::TimeZone;

    fn sample_conn() -> Connection {
        let mut conn = db::open(std::path::Path::new(":memory:")).unwrap();
        let base = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();

        let usage = vec![
            UsageRecord {
                provider: "openai".to_string(),
                model: "gpt-4o-mini".to_string(),
                bucket_start: base,
                bucket_end: base + chrono::Duration::hours(1),
                input_tokens: 1000,
                output_tokens: 500,
                cached_input_tokens: 200,
                cache_creation_tokens: 0,
                requests: 10,
                cost_usd: None,
                source: "openai-usage-api".to_string(),
            },
            UsageRecord {
                provider: "anthropic".to_string(),
                model: "claude-sonnet-4".to_string(),
                bucket_start: base + chrono::Duration::hours(2),
                bucket_end: base + chrono::Duration::hours(3),
                input_tokens: 2000,
                output_tokens: 800,
                cached_input_tokens: 0,
                cache_creation_tokens: 100,
                requests: 5,
                cost_usd: None,
                source: "anthropic-usage-api".to_string(),
            },
        ];
        db::upsert_usage_records(&mut conn, &usage).unwrap();

        let cost = vec![crate::models::CostRecord {
            provider: "anthropic".to_string(),
            model: Some("claude-sonnet-4".to_string()),
            bucket_start: base,
            bucket_end: base + chrono::Duration::days(1),
            amount_usd: 1.23,
            description: Some("Claude Sonnet 4 Usage".to_string()),
            source: "anthropic-cost-api".to_string(),
        }];
        db::upsert_cost_records(&mut conn, &cost).unwrap();

        conn
    }

    #[test]
    fn overview_totals_sum_across_providers() {
        let conn = sample_conn();
        let since = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
        let until = Utc.with_ymd_and_hms(2026, 6, 2, 0, 0, 0).unwrap();
        let cfg = PricingConfig::default();

        let report = build_report(&conn, since, until, None, Granularity::Hour, &cfg).unwrap();

        assert_eq!(report.overview.total_requests, 15);
        assert_eq!(report.overview.total_input_tokens, 3000);
        assert_eq!(report.overview.total_output_tokens, 1300);
        assert_eq!(report.overview.total_tokens, 4300);
        // Anthropic has a reported cost (1.23); OpenAI has none and falls
        // back to the pricing-table estimate.
        assert!(report.overview.total_cost_usd > 1.23);
        assert!(report.overview.cost_partially_estimated);
        assert_eq!(report.overview.providers.len(), 2);
    }

    #[test]
    fn provider_filter_restricts_results() {
        let conn = sample_conn();
        let since = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
        let until = Utc.with_ymd_and_hms(2026, 6, 2, 0, 0, 0).unwrap();
        let cfg = PricingConfig::default();

        let report = build_report(
            &conn,
            since,
            until,
            Some("anthropic"),
            Granularity::Hour,
            &cfg,
        )
        .unwrap();

        assert_eq!(report.by_provider.len(), 1);
        assert_eq!(report.by_provider[0].provider, "anthropic");
        assert_eq!(report.overview.total_requests, 5);
    }

    #[test]
    fn anthropic_cost_is_attached_to_its_model_not_estimated() {
        let conn = sample_conn();
        let since = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
        let until = Utc.with_ymd_and_hms(2026, 6, 2, 0, 0, 0).unwrap();
        let cfg = PricingConfig::default();

        let report = build_report(&conn, since, until, None, Granularity::Hour, &cfg).unwrap();

        let anthropic_model = report
            .by_model
            .iter()
            .find(|m| m.provider == "anthropic")
            .unwrap();
        assert_eq!(anthropic_model.cost_usd, Some(1.23));
        assert!(!anthropic_model.cost_estimated);

        let openai_model = report
            .by_model
            .iter()
            .find(|m| m.provider == "openai")
            .unwrap();
        assert!(openai_model.cost_estimated);
        assert!(openai_model.cost_usd.is_some());
    }

    #[test]
    fn time_series_buckets_by_requested_granularity() {
        let conn = sample_conn();
        let since = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
        let until = Utc.with_ymd_and_hms(2026, 6, 2, 0, 0, 0).unwrap();
        let cfg = PricingConfig::default();

        let hourly = build_report(&conn, since, until, None, Granularity::Hour, &cfg).unwrap();
        assert_eq!(hourly.time_series.len(), 2); // two distinct hour buckets

        let daily = build_report(&conn, since, until, None, Granularity::Day, &cfg).unwrap();
        assert_eq!(daily.time_series.len(), 1); // both fall on the same day
        assert_eq!(daily.time_series[0].total_tokens, 4300);
    }
}
