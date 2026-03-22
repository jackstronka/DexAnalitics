//! Parse Orca Whirlpool `Traded` Anchor events from transaction `logMessages`.
//!
//! Source (on-chain): `orca-so/whirlpools` → `programs/whirlpool/src/events.rs` — `#[event] pub struct Traded`.
//! Logs use `Program data: <base64>` with 8-byte Anchor event discriminator + Borsh payload.

use borsh::{BorshDeserialize, BorshSerialize};
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

/// `sha256("event:Traded")[..8]` (Anchor event discriminator).
pub const TRADED_EVENT_DISCRIMINATOR: [u8; 8] = [225, 202, 73, 175, 147, 43, 160, 150];

/// On-chain `Traded` event fields (Borsh layout must match the program).
#[derive(Debug, Clone, PartialEq, Eq, BorshDeserialize, BorshSerialize)]
pub struct WhirlpoolTradedEvent {
    /// Whirlpool pool pubkey (32 bytes).
    pub whirlpool: [u8; 32],
    pub a_to_b: bool,
    pub pre_sqrt_price: u128,
    pub post_sqrt_price: u128,
    pub input_amount: u64,
    pub output_amount: u64,
    pub input_transfer_fee: u64,
    pub output_transfer_fee: u64,
    pub lp_fee: u64,
    pub protocol_fee: u64,
}

/// Decode the first `Traded` event in logs whose `whirlpool` matches `pool_address`.
#[must_use]
pub fn parse_traded_event_for_pool(logs: &[String], pool_address: &str) -> Option<WhirlpoolTradedEvent> {
    let pool = Pubkey::from_str(pool_address).ok()?;
    for line in logs {
        let rest = line
            .strip_prefix("Program data: ")
            .or_else(|| line.strip_prefix("Program Data: "))
            .map(str::trim)?;
        let bytes = base64_decode(rest)?;
        if bytes.len() < 8 + 32 {
            continue;
        }
        if bytes[..8] != TRADED_EVENT_DISCRIMINATOR {
            continue;
        }
        let mut payload = &bytes[8..];
        let ev = WhirlpoolTradedEvent::deserialize(&mut payload).ok()?;
        if Pubkey::new_from_array(ev.whirlpool) == pool {
            return Some(ev);
        }
    }
    None
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(s.trim()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn roundtrip_synthetic_traded_payload() {
        let pool = Pubkey::new_unique();
        let ev = WhirlpoolTradedEvent {
            whirlpool: pool.to_bytes(),
            a_to_b: true,
            pre_sqrt_price: 1u128 << 64,
            post_sqrt_price: (1u128 << 64) + 100,
            input_amount: 1_000_000,
            output_amount: 2_000_000,
            input_transfer_fee: 0,
            output_transfer_fee: 0,
            lp_fee: 3000,
            protocol_fee: 1000,
        };
        let mut buf = TRADED_EVENT_DISCRIMINATOR.to_vec();
        buf.extend_from_slice(&borsh::to_vec(&ev).unwrap());

        let b64 = base64::engine::general_purpose::STANDARD.encode(&buf);
        let line = format!("Program data: {b64}");
        let parsed = parse_traded_event_for_pool(&[line], &pool.to_string()).expect("parse");
        assert_eq!(parsed, ev);
    }
}
