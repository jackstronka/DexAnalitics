//! RPC provider abstraction with health checks and fallback.
//!
//! This module provides a robust RPC client with:
//! - Multiple endpoint support with automatic fallback
//! - Health checking and endpoint rotation
//! - Rate limiting
//! - Retry logic with exponential backoff

mod cluster;
mod config;
mod health;
mod provider;

pub use cluster::*;
pub use config::*;
pub use health::*;
pub use provider::*;
