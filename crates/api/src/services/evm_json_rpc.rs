//! Minimal Base / EVM JSON-RPC (`eth_call`) for read-only pool state.

use primitive_types::U256;
use serde::Deserialize;
use std::sync::LazyLock;
use std::time::Duration;

/// `slot0()` selector on Uniswap v3–style pools (used by Aerodrome Slipstream).
pub const SLOT0_SELECTOR: &str = "0x3850c7bd";

static HTTP: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("reqwest client for EVM JSON-RPC")
});

#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    result: Option<String>,
    error: Option<JsonRpcErrorObj>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcErrorObj {
    message: Option<String>,
    code: Option<i64>,
}

/// Executes `eth_call` at `block` tag (e.g. `"latest"`).
pub async fn eth_call(rpc_url: &str, to: &str, data: &str, block: &str) -> Result<Vec<u8>, String> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1u64,
        "method": "eth_call",
        "params": [
            {"to": to, "data": data},
            block
        ]
    });

    let resp = HTTP
        .post(rpc_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("BASE RPC request failed: {e}"))?;

    let status = resp.status();
    let v: JsonRpcResponse = resp
        .json()
        .await
        .map_err(|e| format!("BASE RPC invalid JSON (HTTP {status}): {e}"))?;

    if let Some(err) = v.error {
        let msg = err
            .message
            .unwrap_or_else(|| format!("code {:?}", err.code));
        return Err(format!("BASE RPC error: {msg}"));
    }

    let hex = v
        .result
        .ok_or_else(|| "BASE RPC missing result".to_string())?;
    decode_hex_bytes(&hex)
}

fn decode_hex_bytes(hex: &str) -> Result<Vec<u8>, String> {
    let s = hex.strip_prefix("0x").unwrap_or(hex);
    if !s.len().is_multiple_of(2) {
        return Err("odd-length hex".into());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("hex decode: {e}"))
}

/// Decodes ABI-encoded return tuple of `slot0()` (7 × 32-byte words).
pub fn decode_slot0_return(bytes: &[u8]) -> Result<Slot0Decoded, String> {
    if bytes.len() < 224 {
        return Err(format!(
            "slot0 return too short: {} bytes (expected 224)",
            bytes.len()
        ));
    }

    let sqrt = U256::from_big_endian(&bytes[0..32]);
    let tick = word_low24_as_i32(&bytes[32..64]);
    // Words 2–6: observationIndex, observationCardinality, observationCardinalityNext, feeProtocol, unlocked
    let obs_idx = u16::from_be_bytes([bytes[94], bytes[95]]);
    let obs_card = u16::from_be_bytes([bytes[126], bytes[127]]);
    let obs_card_next = u16::from_be_bytes([bytes[158], bytes[159]]);
    let fee_protocol = bytes[191];
    let unlocked = bytes[223] != 0;

    Ok(Slot0Decoded {
        sqrt_price_x96: sqrt,
        tick,
        observation_index: obs_idx,
        observation_cardinality: obs_card,
        observation_cardinality_next: obs_card_next,
        fee_protocol,
        unlocked,
    })
}

fn word_low24_as_i32(word32: &[u8]) -> i32 {
    debug_assert_eq!(word32.len(), 32);
    let raw24 = ((word32[29] as u32) << 16) | ((word32[30] as u32) << 8) | (word32[31] as u32);
    if raw24 & 0x0080_0000 != 0 {
        (raw24 | 0xFF00_0000) as i32
    } else {
        raw24 as i32
    }
}

#[derive(Debug, Clone)]
pub struct Slot0Decoded {
    pub sqrt_price_x96: U256,
    pub tick: i32,
    pub observation_index: u16,
    pub observation_cardinality: u16,
    pub observation_cardinality_next: u16,
    pub fee_protocol: u8,
    pub unlocked: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_slot0_zero_tick() {
        let mut b = vec![0u8; 224];
        b[31] = 1;
        let d = decode_slot0_return(&b).unwrap();
        assert_eq!(d.sqrt_price_x96, U256::from(1u64));
        assert_eq!(d.tick, 0);
        assert!(!d.unlocked);
    }

    #[test]
    fn decode_slot0_negative_tick_sign_ext() {
        let mut b = vec![0u8; 224];
        // Word 1 (bytes 32..64): int24 `-100` sign-extended → last three bytes `FF FF 9C`.
        b[61] = 0xff;
        b[62] = 0xff;
        b[63] = 0x9c;
        let d = decode_slot0_return(&b).unwrap();
        assert_eq!(d.tick, -100);
    }
}
