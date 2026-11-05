use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::config::{self, Config};

pub fn path(config_path: &Path) -> Result<()> {
    println!("{}", config_path.display());
    Ok(())
}

pub fn show(config_path: &Path) -> Result<()> {
    let cfg = Config::load_or_default(config_path)?;
    let masked = config::masked(&cfg);
    println!("{}", serde_yaml::to_string(&masked)?);
    Ok(())
}

/// Sets a dotted config key (e.g. `providers.openai.enabled`) to a value,
/// parsing booleans/numbers/null where they match, otherwise storing a
/// string. Operates on the generic YAML tree so it works uniformly across
/// the whole config schema, then round-trips through `Config` to validate
/// the result still deserializes correctly.
pub fn set(config_path: &Path, key: &str, value: &str) -> Result<()> {
    let cfg = Config::load_or_default(config_path)?;
    let mut tree: serde_yaml::Value = serde_yaml::to_value(&cfg)?;

    let segments: Vec<&str> = key.split('.').collect();
    if segments.is_empty() || segments.iter().any(|s| s.is_empty()) {
        bail!("invalid key '{key}' (expected dotted path, e.g. providers.openai.enabled)");
    }
    set_path(&mut tree, &segments, value)?;

    let updated: Config = serde_yaml::from_value(tree)
        .context("resulting config is no longer valid; check the key/value")?;
    updated.save(config_path)?;
    println!("Set {key} = {value}");
    Ok(())
}

fn set_path(node: &mut serde_yaml::Value, path: &[&str], raw: &str) -> Result<()> {
    if !node.is_mapping() {
        *node = serde_yaml::Value::Mapping(Default::default());
    }
    let mapping = node.as_mapping_mut().expect("just ensured mapping");
    let key = serde_yaml::Value::String(path[0].to_string());

    if path.len() == 1 {
        mapping.insert(key, parse_scalar(raw));
        return Ok(());
    }

    if !mapping.contains_key(&key) {
        mapping.insert(key.clone(), serde_yaml::Value::Mapping(Default::default()));
    }
    let child = mapping.get_mut(&key).expect("just inserted");
    set_path(child, &path[1..], raw)
}

fn parse_scalar(raw: &str) -> serde_yaml::Value {
    match raw {
        "true" => serde_yaml::Value::Bool(true),
        "false" => serde_yaml::Value::Bool(false),
        "null" | "~" => serde_yaml::Value::Null,
        _ => {
            if let Ok(i) = raw.parse::<i64>() {
                serde_yaml::Value::Number(i.into())
            } else if let Ok(f) = raw.parse::<f64>() {
                serde_yaml::Value::Number(f.into())
            } else {
                serde_yaml::Value::String(raw.to_string())
            }
        }
    }
}
