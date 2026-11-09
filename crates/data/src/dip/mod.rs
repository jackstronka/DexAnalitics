//! Dip Agent helpers (zones + Trigger read + propose envelope).
//!
//! Docs: `doc/dip-agent/`. Config seed: `config/dip/`.

mod propose;
mod zones;

pub use propose::{
    DIP_PROPOSE_KIND, DIP_PROPOSE_SOURCE, DipAuditDecisionRow, DipProposeAction, DipProposeDecision,
    DipProposeMode, append_dip_propose, default_agent_decisions_path,
};
pub use zones::{
    DipZoneBand, DipZoneLabel, DipZonesFile, SymbolZones, classify_zone, default_sol_zones,
    default_zones_file, load_dip_zones_file,
};
