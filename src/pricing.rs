//! Fallback cost estimation for usage records that don't carry a cost value.
//!
//! Real costs from the OpenAI Costs API and the Anthropic Cost Report are
//! always preferred and stored as-is. This table only kicks in when a source
//! gives us token counts but no cost (e.g. a CSV import, or a provider with
//! no billing API). Prices drift constantly, so this table intentionally
//! stays small and every lookup can be overridden in `config.yaml` under
//! `pricing.overrides`. Local inference (Ollama/LM Studio) is always free.

use crate::config::PricingConfig;

pub struct Price {
    pub input_per_million: f64,
    pub output_per_million: f64,
    pub cached_input_per_million: Option<f64>,
}

/// Small built-in table of approximate list prices (USD per 1,000,000
/// tokens) for a handful of widely used hosted models, current as of the
/// tool's release. These are estimates only — always verify against the
/// provider's official pricing page, or set `pricing.overrides` in
/// config.yaml for accurate figures.
const BUILTIN_TABLE: &[(&str, f64, f64, Option<f64>)] = &[
    // OpenAI
    ("gpt-5", 1.25, 10.0, Some(0.125)),
    ("gpt-5-mini", 0.25, 2.0, Some(0.025)),
    ("gpt-5-nano", 0.05, 0.4, Some(0.005)),
    ("gpt-4o", 2.5, 10.0, Some(1.25)),
    ("gpt-4o-mini", 0.15, 0.6, Some(0.075)),
    ("o3", 2.0, 8.0, Some(0.5)),
    ("o4-mini", 1.1, 4.4, Some(0.275)),
    // Anthropic
    ("claude-opus-4", 15.0, 75.0, Some(1.5)),
    ("claude-sonnet-4", 3.0, 15.0, Some(0.3)),
    ("claude-haiku-4", 1.0, 5.0, Some(0.1)),
    ("claude-3-5-sonnet", 3.0, 15.0, Some(0.3)),
    ("claude-3-5-haiku", 0.8, 4.0, Some(0.08)),
];

pub fn lookup(model: &str, pricing_cfg: &PricingConfig) -> Option<Price> {
    if let Some(o) = pricing_cfg.overrides.get(model) {
        return Some(Price {
            input_per_million: o.input_per_million,
            output_per_million: o.output_per_million,
            cached_input_per_million: o.cached_input_per_million,
        });
    }

    let needle = model.to_ascii_lowercase();
    // Pick the longest matching table key rather than the first one found,
    // so a more specific name (e.g. "gpt-4o-mini") wins over a shorter
    // prefix that would otherwise match first (e.g. "gpt-4o").
    BUILTIN_TABLE
        .iter()
        .filter(|(name, ..)| needle.starts_with(name) || needle.contains(name))
        .max_by_key(|(name, ..)| name.len())
        .map(|(_, i, o, c)| Price {
            input_per_million: *i,
            output_per_million: *o,
            cached_input_per_million: *c,
        })
}

/// Estimates a USD cost for the given token counts, returning `None` when no
/// pricing entry is known for the model (rather than silently guessing $0).
pub fn estimate_cost_usd(
    model: &str,
    input_tokens: i64,
    output_tokens: i64,
    cached_input_tokens: i64,
    pricing_cfg: &PricingConfig,
) -> Option<f64> {
    let price = lookup(model, pricing_cfg)?;
    let billable_input = (input_tokens - cached_input_tokens).max(0) as f64;
    let cached = cached_input_tokens as f64;
    let cached_rate = price
        .cached_input_per_million
        .unwrap_or(price.input_per_million);

    let cost = (billable_input * price.input_per_million
        + cached * cached_rate
        + output_tokens as f64 * price.output_per_million)
        / 1_000_000.0;
    Some(cost)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_uses_builtin_table_for_known_model() {
        let cfg = PricingConfig::default();
        let cost = estimate_cost_usd("gpt-4o-mini", 1_000_000, 1_000_000, 0, &cfg).unwrap();
        // 0.15 (input) + 0.6 (output) per the builtin table.
        assert!((cost - 0.75).abs() < 1e-9);
    }

    #[test]
    fn estimate_returns_none_for_unknown_model() {
        let cfg = PricingConfig::default();
        assert!(estimate_cost_usd("some-unknown-model-xyz", 1000, 1000, 0, &cfg).is_none());
    }

    #[test]
    fn cached_tokens_are_not_double_billed_at_full_input_rate() {
        let cfg = PricingConfig::default();
        // All input tokens are cached: should bill at the cached rate only.
        let cost = estimate_cost_usd("gpt-4o-mini", 1_000_000, 0, 1_000_000, &cfg).unwrap();
        assert!((cost - 0.075).abs() < 1e-9);
    }

    #[test]
    fn override_takes_priority_over_builtin_table() {
        let mut cfg = PricingConfig::default();
        cfg.overrides.insert(
            "gpt-4o-mini".to_string(),
            crate::config::ModelPriceOverride {
                input_per_million: 100.0,
                output_per_million: 200.0,
                cached_input_per_million: None,
            },
        );
        let cost = estimate_cost_usd("gpt-4o-mini", 1_000_000, 0, 0, &cfg).unwrap();
        assert!((cost - 100.0).abs() < 1e-9);
    }
}
