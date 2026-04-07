//! Rolling indicators for backtest strategies (Bollinger, last-completed candle index).

/// Population standard deviation and mean of `xs` (empty → `None`).
#[must_use]
pub fn mean_std(xs: &[f64]) -> Option<(f64, f64)> {
    let n = xs.len();
    if n == 0 {
        return None;
    }
    let mean = xs.iter().sum::<f64>() / n as f64;
    let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n as f64;
    let std = var.sqrt();
    Some((mean, std))
}

/// Bollinger lower/upper from closes, `lower = sma - k*σ`, `upper = sma + k*σ`.
/// If bands are invalid (non-positive lower or inverted), returns `None`.
#[must_use]
pub fn bollinger_lower_upper(closes: &[f64], k: f64) -> Option<(f64, f64)> {
    let (sma, std) = mean_std(closes)?;
    let lo = sma - k * std;
    let hi = sma + k * std;
    if lo > 0.0 && lo < hi {
        Some((lo, hi))
    } else {
        None
    }
}

/// Index into `step_data` for the **close** of the last **fully completed** candle when each
/// candle spans `candle_steps` simulation steps (see `doc/IMPLEMENTATION_PLAN_BOLLINGER_CANDLE_STRATEGIES.md`).
#[must_use]
pub fn last_closed_candle_close_idx(i: usize, candle_steps: usize) -> usize {
    let cs = candle_steps.max(1);
    let num_complete = (i + 1) / cs;
    if num_complete == 0 {
        return 0;
    }
    let last_candle_id = num_complete - 1;
    last_candle_id * cs + cs - 1
}

/// Inclusive `[start, end]` step indices of the **last fully completed** candle at simulation step `i`.
/// `None` if no candle has closed yet (`i + 1 < candle_steps`).
#[must_use]
pub fn last_closed_candle_step_range(i: usize, candle_steps: usize) -> Option<(usize, usize)> {
    let cs = candle_steps.max(1);
    let num_complete = (i + 1) / cs;
    if num_complete == 0 {
        return None;
    }
    let last_candle_id = num_complete - 1;
    let start = last_candle_id * cs;
    let end = last_candle_id * cs + cs - 1;
    debug_assert!(end <= i);
    Some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mean_std_matches_three_point_manual() {
        let xs = [1.0_f64, 2.0, 3.0];
        let (m, s) = mean_std(&xs).unwrap();
        assert!((m - 2.0).abs() < 1e-9);
        let expected_var =
            ((1.0_f64 - 2.0).powi(2) + (2.0_f64 - 2.0).powi(2) + (3.0_f64 - 2.0).powi(2)) / 3.0;
        assert!((s - expected_var.sqrt()).abs() < 1e-9);
    }

    #[test]
    fn bollinger_positive_band() {
        // Tight positive levels so `sma - 2σ` stays > 0 (linear 1..30 would put lower band below zero).
        let xs: Vec<f64> = (0..30).map(|i| 100.0 + (i as f64) * 0.05).collect();
        let (lo, hi) = bollinger_lower_upper(&xs, 2.0).expect("bands");
        assert!(lo < hi && lo > 0.0);
    }

    #[test]
    fn last_closed_candle_idx_step4() {
        assert_eq!(last_closed_candle_close_idx(0, 4), 0);
        assert_eq!(last_closed_candle_close_idx(3, 4), 3);
        assert_eq!(last_closed_candle_close_idx(4, 4), 3);
        assert_eq!(last_closed_candle_close_idx(7, 4), 7);
    }

    #[test]
    fn last_closed_candle_step_range_matches_candle() {
        assert_eq!(last_closed_candle_step_range(2, 4), None);
        assert_eq!(last_closed_candle_step_range(3, 4), Some((0, 3)));
        assert_eq!(last_closed_candle_step_range(4, 4), Some((0, 3)));
        assert_eq!(last_closed_candle_step_range(7, 4), Some((4, 7)));
    }
}
