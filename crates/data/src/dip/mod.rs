//! Dip Agent helpers (zones + Trigger read + propose envelope + runtime).
//!
//! Docs: `doc/dip-agent/`. Config seed: `config/dip/`.

mod propose;
mod runtime;
mod zones;

pub use propose::{
    DIP_PROPOSE_KIND, DIP_PROPOSE_SOURCE, DipAuditDecisionRow, DipProposeAction,
    DipProposeDecision, DipProposeMode, append_dip_propose, default_agent_decisions_path,
};
pub use runtime::{
    DipLadderDefaults, DipRuntimeFile, default_runtime_path, load_dip_runtime_file,
    normalize_dip_mode, resolve_runtime_path,
};
pub use zones::{
    DipZoneBand, DipZoneLabel, DipZonesFile, SymbolZones, classify_zone, default_sol_zones,
    default_zones_file, load_dip_zones_file,
};
