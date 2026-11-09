//! Price zones A/B/C for Dip Agent (from `config/dip/zones.json` or defaults).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Inclusive USD band for a strategy zone.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct DipZoneBand {
    pub min: f64,
    pub max: f64,
}

/// Zones A/B/C for one symbol.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SymbolZones {
    pub a: DipZoneBand,
    pub b: DipZoneBand,
    pub c: DipZoneBand,
}

/// On-disk zones file (`config/dip/zones.json`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DipZonesFile {
    pub version: u32,
    pub symbols: BTreeMap<String, SymbolZones>,
}

/// Classified zone label for a spot price.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DipZoneLabel {
    A,
    B,
    C,
    None,
}

impl DipZoneLabel {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::C => "C",
            Self::None => "none",
        }
    }
}

/// Defaults matching `config/dip/mints.yaml` zones for SOL (2026-11-04 seed).
#[must_use]
pub fn default_sol_zones() -> SymbolZones {
    SymbolZones {
        a: DipZoneBand {
            min: 91.5,
            max: 93.5,
        },
        b: DipZoneBand {
            min: 88.0,
            max: 90.5,
        },
        c: DipZoneBand {
            min: 84.0,
            max: 87.0,
        },
    }
}

#[must_use]
pub fn default_zones_file() -> DipZonesFile {
    let mut symbols = BTreeMap::new();
    symbols.insert("SOL".into(), default_sol_zones());
    DipZonesFile {
        version: 1,
        symbols,
    }
}

/// Load zones JSON; missing file → defaults (SOL only).
pub fn load_dip_zones_file(path: &Path) -> Result<DipZonesFile> {
    if !path.exists() {
        return Ok(default_zones_file());
    }
    let raw =
        fs::read_to_string(path).with_context(|| format!("read dip zones {}", path.display()))?;
    let file: DipZonesFile = serde_json::from_str(&raw)
        .with_context(|| format!("parse dip zones {}", path.display()))?;
    Ok(file)
}

/// Classify `price_usd` into A/B/C for `symbol` (case-insensitive). Overlaps prefer A then B then C.
#[must_use]
pub fn classify_zone(zones: &DipZonesFile, symbol: &str, price_usd: f64) -> DipZoneLabel {
    let Some(z) = zones
        .symbols
        .get(symbol)
        .or_else(|| zones.symbols.get(&symbol.to_uppercase()))
    else {
        return DipZoneLabel::None;
    };
    if in_band(price_usd, z.a) {
        return DipZoneLabel::A;
    }
    if in_band(price_usd, z.b) {
        return DipZoneLabel::B;
    }
    if in_band(price_usd, z.c) {
        return DipZoneLabel::C;
    }
    DipZoneLabel::None
}

fn in_band(price: f64, band: DipZoneBand) -> bool {
    price >= band.min && price <= band.max
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_sol_bands() {
        let z = default_zones_file();
        assert_eq!(classify_zone(&z, "SOL", 92.0), DipZoneLabel::A);
        assert_eq!(classify_zone(&z, "SOL", 89.0), DipZoneLabel::B);
        assert_eq!(classify_zone(&z, "SOL", 85.0), DipZoneLabel::C);
        assert_eq!(classify_zone(&z, "SOL", 95.0), DipZoneLabel::None);
    }

    #[test]
    fn load_missing_uses_defaults() {
        let f = load_dip_zones_file(Path::new("no/such/zones.json")).unwrap();
        assert!(f.symbols.contains_key("SOL"));
    }
}
