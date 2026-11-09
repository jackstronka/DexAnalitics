//! Dip Agent runtime (`config/dip/runtime.yaml`) — public fields only (no secrets).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Ladder form defaults (3Commas-like model A). Optional section in `runtime.yaml`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DipLadderDefaults {
    #[serde(default = "default_base_order_usd")]
    pub base_order_usd: f64,
    #[serde(default = "default_safety_order_usd")]
    pub safety_order_usd: f64,
    #[serde(default = "default_deviation_pct")]
    pub deviation_pct: f64,
    #[serde(default = "default_deviation_step_scale")]
    pub deviation_step_scale: f64,
    #[serde(default = "default_volume_scale")]
    pub volume_scale: f64,
    #[serde(default = "default_max_safety_orders")]
    pub max_safety_orders: u32,
    #[serde(default = "default_take_profit_pct")]
    pub take_profit_pct: f64,
    #[serde(default = "default_min_order_usd")]
    pub min_order_usd: f64,
}

fn default_base_order_usd() -> f64 {
    50.0
}
fn default_safety_order_usd() -> f64 {
    50.0
}
fn default_deviation_pct() -> f64 {
    1.0
}
fn default_deviation_step_scale() -> f64 {
    1.0
}
fn default_volume_scale() -> f64 {
    1.5
}
fn default_max_safety_orders() -> u32 {
    5
}
fn default_take_profit_pct() -> f64 {
    1.5
}
fn default_min_order_usd() -> f64 {
    10.0
}

impl Default for DipLadderDefaults {
    fn default() -> Self {
        Self {
            base_order_usd: default_base_order_usd(),
            safety_order_usd: default_safety_order_usd(),
            deviation_pct: default_deviation_pct(),
            deviation_step_scale: default_deviation_step_scale(),
            volume_scale: default_volume_scale(),
            max_safety_orders: default_max_safety_orders(),
            take_profit_pct: default_take_profit_pct(),
            min_order_usd: default_min_order_usd(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct JupiterRuntimeYaml {
    #[serde(default)]
    pub dca_allowed: bool,
}

/// Subset of `config/dip/runtime.yaml` needed by Settings / UI (U2).
#[derive(Debug, Clone, Deserialize)]
pub struct DipRuntimeFile {
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default = "default_jupiter")]
    pub jupiter: JupiterRuntimeYaml,
    #[serde(default)]
    pub ladder: DipLadderDefaults,
}

fn default_mode() -> String {
    "propose-only".into()
}

fn default_jupiter() -> JupiterRuntimeYaml {
    JupiterRuntimeYaml { dca_allowed: true }
}

impl Default for DipRuntimeFile {
    fn default() -> Self {
        Self {
            mode: default_mode(),
            jupiter: JupiterRuntimeYaml { dca_allowed: true },
            ladder: DipLadderDefaults::default(),
        }
    }
}

#[must_use]
pub fn default_runtime_path() -> PathBuf {
    PathBuf::from("config/dip/runtime.yaml")
}

/// Resolve runtime path: override → `CLMM_DIP_RUNTIME_PATH` → `config/dip/runtime.yaml`.
#[must_use]
pub fn resolve_runtime_path(override_path: Option<&str>) -> PathBuf {
    if let Some(p) = override_path.filter(|s| !s.trim().is_empty()) {
        return PathBuf::from(p);
    }
    if let Ok(p) = std::env::var("CLMM_DIP_RUNTIME_PATH")
        && !p.trim().is_empty()
    {
        return PathBuf::from(p);
    }
    default_runtime_path()
}

pub fn load_dip_runtime_file(path: &Path) -> Result<DipRuntimeFile> {
    let raw =
        fs::read_to_string(path).with_context(|| format!("read dip runtime {}", path.display()))?;
    let file: DipRuntimeFile = serde_yaml::from_str(&raw)
        .with_context(|| format!("parse dip runtime yaml {}", path.display()))?;
    Ok(file)
}

/// Normalize mode string to a known token; unknown values pass through trimmed.
#[must_use]
pub fn normalize_dip_mode(mode: &str) -> String {
    let m = mode.trim().to_ascii_lowercase();
    match m.as_str() {
        "propose-only" | "propose_only" | "proposeonly" => "propose-only".into(),
        "paper" => "paper".into(),
        "live" => "live".into(),
        _ => mode.trim().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn loads_runtime_yaml_with_ladder_defaults() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("runtime.yaml");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"
mode: paper
jupiter:
  dca_allowed: false
ladder:
  base_order_usd: 40
  safety_order_usd: 20
  deviation_pct: 2
  deviation_step_scale: 1.2
  volume_scale: 2
  max_safety_orders: 3
  take_profit_pct: 1.25
  min_order_usd: 10
"#
        )
        .unwrap();
        let rt = load_dip_runtime_file(&path).unwrap();
        assert_eq!(normalize_dip_mode(&rt.mode), "paper");
        assert!(!rt.jupiter.dca_allowed);
        assert!((rt.ladder.base_order_usd - 40.0).abs() < 1e-9);
        assert!((rt.ladder.volume_scale - 2.0).abs() < 1e-9);
        assert_eq!(rt.ladder.max_safety_orders, 3);
    }

    #[test]
    fn missing_ladder_uses_defaults() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("runtime.yaml");
        fs::write(&path, "mode: propose-only\njupiter:\n  dca_allowed: true\n").unwrap();
        let rt = load_dip_runtime_file(&path).unwrap();
        assert_eq!(normalize_dip_mode(&rt.mode), "propose-only");
        assert_eq!(rt.ladder, DipLadderDefaults::default());
    }
}
