//! Regression for `snapshot_readiness` tier-2 output. Uses **minimal synthetic JSONL** in a temp
//! directory (no committed snapshot files under `data/`).

use std::path::PathBuf;
use std::process::Command;

/// Two rows with `ts_utc`, mints, and Raydium fee-growth fields — satisfies tier 2 heuristic.
const MIN_RAYDIUM_JSONL: &str = r#"{"ts_utc":"2026-01-01T00:00:00+00:00","token_mint_a":"So11111111111111111111111111111111111111112","token_mint_b":"Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB","fee_growth_global_a_x64":"1","fee_growth_global_b_x64":"2"}
{"ts_utc":"2026-01-01T00:01:00+00:00","token_mint_a":"So11111111111111111111111111111111111111112","token_mint_b":"Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB","fee_growth_global_a_x64":"1","fee_growth_global_b_x64":"2"}
"#;

/// Meteora ignores fee_growth in tier-2; needs `protocol_fee_amount_a/b` on ≥2 rows.
const MIN_METEORA_JSONL: &str = r#"{"ts_utc":"2026-01-01T00:00:00+00:00","token_mint_a":"GcdayuLaLysEhreF7fxonjrXE5kVacdDTAEu3ua1af6","token_mint_b":"BGoJdfAA39yRXrBJrJbeU1on8Y2durbCt2m5YLrFSKb1","protocol_fee_amount_a":12675790170239661953,"protocol_fee_amount_b":810333675}
{"ts_utc":"2026-01-01T00:01:00+00:00","token_mint_a":"GcdayuLaLysEhreF7fxonjrXE5kVacdDTAEu3ua1af6","token_mint_b":"BGoJdfAA39yRXrBJrJbeU1on8Y2durbCt2m5YLrFSKb1","protocol_fee_amount_a":12675790170239661953,"protocol_fee_amount_b":810333675}
"#;

struct CleanupTempDir(PathBuf);

impl Drop for CleanupTempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn temp_cwd_with_snapshot(protocol: &str, pool_address: &str, jsonl: &str) -> PathBuf {
    let uniq = format!(
        "clmm_snapshot_readiness_{}_{}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
        protocol,
        pool_address
    );
    let root = std::env::temp_dir().join(uniq);
    let file = root
        .join("data")
        .join("pool-snapshots")
        .join(protocol)
        .join(pool_address)
        .join("snapshots.jsonl");
    std::fs::create_dir_all(file.parent().unwrap()).expect("create_dir_all");
    std::fs::write(&file, jsonl).expect("write snapshot jsonl");
    root
}

fn readiness_stdout(protocol: &str, pool_address: &str, jsonl: &str) -> String {
    let cwd = temp_cwd_with_snapshot(protocol, pool_address, jsonl);
    let _guard = CleanupTempDir(cwd.clone());
    let exe = env!("CARGO_BIN_EXE_snapshot_readiness");
    let out = Command::new(exe)
        .args(["--protocol", protocol, "--pool-address", pool_address])
        .current_dir(&cwd)
        .output()
        .expect("failed to execute snapshot_readiness");

    assert!(
        out.status.success(),
        "snapshot_readiness failed: status={:?}, stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn position_truth_report_stdout(pool: &str, position: &str, ledger_jsonl: &str) -> String {
    let root = temp_cwd_with_position_fee_ledger(ledger_jsonl);
    let _guard = CleanupTempDir(root.clone());
    // Need snapshot file too? No, this is separate bin.
    let exe = env!("CARGO_BIN_EXE_position_truth_report");
    let out = Command::new(exe)
        .args([
            "--pool-address",
            pool,
            "--position-address",
            position,
        ])
        .current_dir(&root)
        .output()
        .expect("failed to execute position_truth_report");

    assert!(
        out.status.success(),
        "position_truth_report failed: status={:?}, stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn position_truth_report_prints_summary() {
    let pool = "5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6";
    let ledger = format!(
        "{{\"schema_version\":1,\"ts_utc\":\"2026-01-01T00:00:00Z\",\"position\":\"P1\",\"pool\":\"{pool}\",\"event_type\":\"open_position\",\"tick_lower\":-10,\"tick_upper\":10,\"liquidity\":\"1\",\"fees_owed_a\":0,\"fees_owed_b\":0,\"collected_a\":5,\"collected_b\":7,\"source\":\"derived\"}}\n"
    );
    let stdout = position_truth_report_stdout(pool, "P1", &ledger);
    assert!(stdout.contains("Position-truth report (MVP):"), "stdout:\n{stdout}");
    assert!(stdout.contains("checkpoints: 1"), "stdout:\n{stdout}");
    assert!(stdout.contains("collected_a_sum: 5"), "stdout:\n{stdout}");
    assert!(stdout.contains("collected_b_sum: 7"), "stdout:\n{stdout}");
}

fn temp_cwd_with_position_fee_ledger(ledger_jsonl: &str) -> PathBuf {
    let uniq = format!(
        "clmm_position_fee_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    );
    let root = std::env::temp_dir().join(uniq);
    let file = root.join("data").join("position-fee-checkpoints.jsonl");
    std::fs::create_dir_all(file.parent().unwrap()).expect("create_dir_all");
    std::fs::write(&file, ledger_jsonl).expect("write position fee ledger jsonl");
    root
}

#[test]
fn tier3_position_truth_ready_for_minimal_checkpoint_fixture() {
    // We still need a snapshot file for the command to run (tiers 1/2 parsing path).
    let protocol = "meteora";
    let pool = "5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6";
    let cwd = temp_cwd_with_snapshot(protocol, pool, MIN_METEORA_JSONL);
    let ledger_root = temp_cwd_with_position_fee_ledger(&format!(
        "{{\"schema_version\":1,\"ts_utc\":\"2026-01-01T00:00:00Z\",\"position\":\"P1\",\"pool\":\"{pool}\",\"event_type\":\"open_position\",\"tick_lower\":-10,\"tick_upper\":10,\"liquidity\":\"1\",\"fees_owed_a\":0,\"fees_owed_b\":0,\"collected_a\":0,\"collected_b\":0,\"source\":\"derived\"}}\n\
{{\"schema_version\":1,\"ts_utc\":\"2026-01-01T00:01:00Z\",\"position\":\"P1\",\"pool\":\"{pool}\",\"event_type\":\"collect_fees\",\"tick_lower\":0,\"tick_upper\":0,\"liquidity\":\"0\",\"fees_owed_a\":0,\"fees_owed_b\":0,\"collected_a\":0,\"collected_b\":0,\"source\":\"missing\"}}\n"
    ));

    // Merge: copy ledger file into the snapshot temp cwd under data/.
    let ledger_src = ledger_root.join("data").join("position-fee-checkpoints.jsonl");
    let ledger_dst = cwd.join("data").join("position-fee-checkpoints.jsonl");
    std::fs::copy(&ledger_src, &ledger_dst).expect("copy ledger");

    let _guard1 = CleanupTempDir(cwd.clone());
    let _guard2 = CleanupTempDir(ledger_root.clone());

    let exe = env!("CARGO_BIN_EXE_snapshot_readiness");
    let out = Command::new(exe)
        .args([
            "--protocol",
            protocol,
            "--pool-address",
            pool,
            "--fee-mode",
            "position-truth",
            "--position-address",
            "P1",
        ])
        .current_dir(&cwd)
        .output()
        .expect("failed to execute snapshot_readiness");

    assert!(
        out.status.success(),
        "snapshot_readiness failed: status={:?}, stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        stdout.contains("3) Position-truth fee model: READY"),
        "unexpected output:\n{stdout}"
    );
}

#[test]
fn tier3_position_truth_autodetects_single_position_when_omitted() {
    let protocol = "meteora";
    let pool = "5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6";
    let cwd = temp_cwd_with_snapshot(protocol, pool, MIN_METEORA_JSONL);
    let ledger_root = temp_cwd_with_position_fee_ledger(&format!(
        "{{\"schema_version\":1,\"ts_utc\":\"2026-01-01T00:00:00Z\",\"position\":\"P_ONLY\",\"pool\":\"{pool}\",\"event_type\":\"open_position\",\"tick_lower\":-10,\"tick_upper\":10,\"liquidity\":\"1\",\"fees_owed_a\":0,\"fees_owed_b\":0,\"collected_a\":0,\"collected_b\":0,\"source\":\"derived\"}}\n\
{{\"schema_version\":1,\"ts_utc\":\"2026-01-01T00:01:00Z\",\"position\":\"P_ONLY\",\"pool\":\"{pool}\",\"event_type\":\"collect_fees\",\"tick_lower\":0,\"tick_upper\":0,\"liquidity\":\"0\",\"fees_owed_a\":0,\"fees_owed_b\":0,\"collected_a\":0,\"collected_b\":0,\"source\":\"missing\"}}\n"
    ));
    let ledger_src = ledger_root.join("data").join("position-fee-checkpoints.jsonl");
    let ledger_dst = cwd.join("data").join("position-fee-checkpoints.jsonl");
    std::fs::copy(&ledger_src, &ledger_dst).expect("copy ledger");

    let _guard1 = CleanupTempDir(cwd.clone());
    let _guard2 = CleanupTempDir(ledger_root.clone());

    let exe = env!("CARGO_BIN_EXE_snapshot_readiness");
    let out = Command::new(exe)
        .args([
            "--protocol",
            protocol,
            "--pool-address",
            pool,
            "--fee-mode",
            "position-truth",
        ])
        .current_dir(&cwd)
        .output()
        .expect("failed to execute snapshot_readiness");

    assert!(
        out.status.success(),
        "snapshot_readiness failed: status={:?}, stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        stdout.contains("3) Position-truth fee model: READY"),
        "unexpected output:\n{stdout}"
    );
}

#[test]
fn tier3_position_truth_prints_suggested_commands_when_multiple_positions() {
    let protocol = "meteora";
    let pool = "5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6";
    let cwd = temp_cwd_with_snapshot(protocol, pool, MIN_METEORA_JSONL);
    let ledger_root = temp_cwd_with_position_fee_ledger(&format!(
        "{{\"schema_version\":1,\"ts_utc\":\"2026-01-01T00:00:00Z\",\"position\":\"P1\",\"pool\":\"{pool}\",\"event_type\":\"open_position\",\"tick_lower\":-10,\"tick_upper\":10,\"liquidity\":\"1\",\"fees_owed_a\":0,\"fees_owed_b\":0,\"collected_a\":0,\"collected_b\":0,\"source\":\"derived\"}}\n\
{{\"schema_version\":1,\"ts_utc\":\"2026-01-01T00:00:10Z\",\"position\":\"P2\",\"pool\":\"{pool}\",\"event_type\":\"open_position\",\"tick_lower\":-10,\"tick_upper\":10,\"liquidity\":\"1\",\"fees_owed_a\":0,\"fees_owed_b\":0,\"collected_a\":0,\"collected_b\":0,\"source\":\"derived\"}}\n"
    ));
    let ledger_src = ledger_root.join("data").join("position-fee-checkpoints.jsonl");
    let ledger_dst = cwd.join("data").join("position-fee-checkpoints.jsonl");
    std::fs::copy(&ledger_src, &ledger_dst).expect("copy ledger");

    let _guard1 = CleanupTempDir(cwd.clone());
    let _guard2 = CleanupTempDir(ledger_root.clone());

    let exe = env!("CARGO_BIN_EXE_snapshot_readiness");
    let out = Command::new(exe)
        .args([
            "--protocol",
            protocol,
            "--pool-address",
            pool,
            "--fee-mode",
            "position-truth",
        ])
        .current_dir(&cwd)
        .output()
        .expect("failed to execute snapshot_readiness");

    assert!(
        out.status.success(),
        "snapshot_readiness failed: status={:?}, stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        stdout.contains("suggested commands (pick one position):"),
        "unexpected output:\n{stdout}"
    );
    assert!(stdout.contains("--position-address P1"), "stdout:\n{stdout}");
    assert!(stdout.contains("--position-address P2"), "stdout:\n{stdout}");
}

#[test]
fn raydium_tier2_is_ready_for_fixture() {
    let stdout = readiness_stdout(
        "raydium",
        "3nMFwZXwY1s1M5s8vYAHqd4wGs4iSxXE4LRoUMMYqEgF",
        MIN_RAYDIUM_JSONL,
    );
    assert!(
        stdout.contains("2) Snapshot fee heuristic (experimental): READY"),
        "unexpected output:\n{stdout}"
    );
}

#[test]
fn meteora_tier2_is_ready_for_fixture() {
    let stdout = readiness_stdout(
        "meteora",
        "5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6",
        MIN_METEORA_JSONL,
    );
    assert!(
        stdout.contains("2) Snapshot fee heuristic (experimental): READY"),
        "unexpected output:\n{stdout}"
    );
}
