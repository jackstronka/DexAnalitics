//! Service layer for API operations.
//!
//! This module provides services that bridge API handlers with
//! the execution layer.

pub mod evm_json_rpc;
pub mod lifecycle_ledger_aggregates;
pub mod optimization_runner;
pub mod orca_read_service;
pub mod orca_tx_service;
pub mod position_agent_llm;
pub mod position_agent_service;
pub mod position_close_all;
pub mod position_close_ops;
pub mod position_close_signer;
pub mod chain_economic_totals;
pub mod position_chain_history;
pub mod position_executor;
pub mod position_service;
pub mod position_stream_lineage;
pub mod position_stream_performance;
pub mod position_stream_pnl;
pub mod position_on_chain_cache;
pub mod position_wallet_store;
pub mod registry_stale_reconcile;
pub mod position_valuation;
pub mod price_fetch;
pub mod simulation_analytics;
pub mod stranded_rebalance_watchdog;
pub mod strategy_service;
pub mod uncollected_fees_cache;
pub mod wallet_gl_posting;
pub mod chain_portfolio;
pub mod wallet_ledger;
pub mod wallet_ledger_lifecycle;
pub mod wallet_ledger_tx;

pub use orca_read_service::OrcaReadService;
pub use orca_tx_service::OrcaTxService;
pub use position_service::PositionService;
pub use strategy_service::StrategyService;
