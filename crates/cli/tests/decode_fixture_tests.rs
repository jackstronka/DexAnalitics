use std::path::PathBuf;

use base64::Engine;

fn read_first_data_b64(fixture_path: &std::path::Path) -> anyhow::Result<Vec<u8>> {
    let content = std::fs::read_to_string(fixture_path)?;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: serde_json::Value = serde_json::from_str(line)?;
        let Some(data_b64) = v.get("data_b64").and_then(|x| x.as_str()) else {
            continue;
        };
        let decoded = base64::engine::general_purpose::STANDARD.decode(data_b64)?;
        return Ok(decoded);
    }

    Err(anyhow::anyhow!(
        "No non-empty JSON record with `data_b64` found in {}",
        fixture_path.display()
    ))
}

#[test]
fn raydium_fixture_decodes_pool_state() -> anyhow::Result<()> {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data/pool-snapshots/raydium/3nMFwZXwY1s1M5s8vYAHqd4wGs4iSxXE4LRoUMMYqEgF/snapshots.jsonl");

    let data = read_first_data_b64(&fixture)?;
    clmm_lp_protocols::raydium::pool_reader::parse_pool_state(&data)?;
    Ok(())
}

#[test]
fn meteora_fixture_decodes_lb_pair() -> anyhow::Result<()> {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../data/pool-snapshots/meteora/5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6/snapshots.jsonl",
    );

    let data = read_first_data_b64(&fixture)?;
    clmm_lp_protocols::meteora::pool_reader::parse_lb_pair(&data)?;
    Ok(())
}

