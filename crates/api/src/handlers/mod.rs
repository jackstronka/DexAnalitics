//! Request handlers for API endpoints.

pub mod aerodrome_slipstream;
pub mod agent;
pub mod analytics;
pub mod backtests;
pub mod bot_activity;
pub mod data;
pub mod health;
pub mod orca;
pub mod orca_onchain;
pub mod phantom_auth;
pub mod pools;
pub mod position_close_all;
pub mod positions;
pub mod prices;
pub mod scripts;
pub mod strategies;
pub mod tx;
pub mod wallets;

#[cfg(test)]
mod devnet_e2e_tests;
#[cfg(test)]
mod devnet_test_harness;
#[cfg(test)]
mod endpoint_coverage_tests;
#[cfg(test)]
mod orca_tests;
#[cfg(test)]
mod phantom_auth_tests;
#[cfg(test)]
mod pools_tests;
#[cfg(test)]
mod tx_tests;

pub use aerodrome_slipstream::*;
pub use agent::*;
pub use analytics::*;
pub use backtests::*;
pub use bot_activity::*;
pub use data::*;
pub use health::*;
pub use orca::*;
pub use orca_onchain::*;
pub use phantom_auth::*;
pub use pools::*;
pub use position_close_all::*;
pub use positions::*;
pub use prices::*;
pub use scripts::*;
pub use strategies::*;
pub use tx::*;
pub use wallets::*;
