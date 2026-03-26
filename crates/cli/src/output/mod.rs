//! Output formatting for CLI.
//!
//! This module provides rich output formatting including tables,
//! charts, and export functionality.

pub mod chart;
pub mod export;
pub mod optimize_report;
pub mod optimize_result_json;
mod reports;
pub mod table;

pub use chart::*;
pub use export::*;
pub use reports::{AnalysisReport, BacktestReport, OptimizationReport, RangeCandidate};
pub use table::*;
