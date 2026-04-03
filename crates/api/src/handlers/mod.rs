//! Request handlers for API endpoints.

pub mod analytics;
pub mod bot_activity;
pub mod health;
pub mod orca;
pub mod orca_onchain;
pub mod phantom_auth;
pub mod prices;
pub mod pools;
pub mod positions;
pub mod scripts;
pub mod wallets;
pub mod strategies;
pub mod tx;

#[cfg(test)]
mod devnet_e2e_tests;
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

pub use analytics::*;
pub use bot_activity::*;
pub use health::*;
pub use orca::*;
pub use orca_onchain::*;
pub use phantom_auth::*;
pub use prices::*;
pub use pools::*;
pub use positions::*;
pub use scripts::*;
pub use wallets::*;
pub use strategies::*;
pub use tx::*;
