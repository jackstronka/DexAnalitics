use std::process::Command;
use std::path::PathBuf;

fn readiness_stdout(protocol: &str, pool_address: &str) -> String {
    let exe = env!("CARGO_BIN_EXE_snapshot_readiness");
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = Command::new(exe)
        .args(["--protocol", protocol, "--pool-address", pool_address])
        .current_dir(&workspace_root)
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

#[test]
fn raydium_tier2_is_ready_for_fixture() {
    let stdout = readiness_stdout(
        "raydium",
        "3nMFwZXwY1s1M5s8vYAHqd4wGs4iSxXE4LRoUMMYqEgF",
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
    );
    assert!(
        stdout.contains("2) Snapshot fee heuristic (experimental): READY"),
        "unexpected output:\n{stdout}"
    );
}

