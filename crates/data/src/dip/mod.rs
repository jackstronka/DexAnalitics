//! Dip Agent helpers (zones + Trigger read wiring).
//!
//! Docs: `doc/dip-agent/`. Config seed: `config/dip/`.

mod zones;

pub use zones::{
    DipZoneBand, DipZoneLabel, DipZonesFile, SymbolZones, classify_zone, default_sol_zones,
    default_zones_file, load_dip_zones_file,
};
