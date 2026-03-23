//! Parse Raydium CLMM `SwapEvent` from transaction `logMessages` (`Program data:` + base64).
//!
//! Layout matches `raydium_clmm::types::SwapEvent` (Borsh) with Anchor 8-byte event discriminator
//! prefix (`sha256("event:SwapEvent")[..8]`).

use borsh::BorshDeserialize;
use raydium_clmm::types::SwapEvent as RaydiumSwapEvent;

/// `sha256("event:SwapEvent")[..8]` (Anchor event discriminator).
pub const SWAP_EVENT_DISCRIMINATOR: [u8; 8] = [64, 198, 205, 232, 38, 8, 113, 226];

/// Decode the first `SwapEvent` in logs whose `pool_state` matches `pool_address`.
#[must_use]
pub fn parse_raydium_swap_event_for_pool(logs: &[String], pool_address: &str) -> Option<RaydiumSwapEvent> {
    for line in logs {
        let rest = line
            .strip_prefix("Program data: ")
            .or_else(|| line.strip_prefix("Program Data: "))
            .map(str::trim)?;
        let bytes = base64_decode(rest)?;
        if bytes.len() < 8 + 32 {
            continue;
        }
        if bytes[..8] != SWAP_EVENT_DISCRIMINATOR {
            continue;
        }
        let mut payload = &bytes[8..];
        let ev = RaydiumSwapEvent::deserialize(&mut payload).ok()?;
        if ev.pool_state.to_string() == pool_address {
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
    use raydium_clmm::types::SwapEvent;
    use solana_pubkey::Pubkey;

    #[test]
    fn roundtrip_synthetic_swap_event_payload() {
        let pool = Pubkey::new_unique();
        let ev = SwapEvent {
            pool_state: pool,
            sender: Pubkey::new_unique(),
            token_account0: Pubkey::new_unique(),
            token_account1: Pubkey::new_unique(),
            amount0: 1_000_000,
            transfer_fee0: 0,
            amount1: 2_000_000,
            transfer_fee1: 0,
            zero_for_one: true,
            sqrt_price_x64: 1u128 << 64,
            liquidity: 0,
            tick: 0,
        };
        let mut buf = SWAP_EVENT_DISCRIMINATOR.to_vec();
        buf.extend_from_slice(&borsh::to_vec(&ev).unwrap());

        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&buf);
        let line = format!("Program data: {b64}");
        let parsed = parse_raydium_swap_event_for_pool(&[line], &pool.to_string()).expect("parse");
        assert_eq!(parsed, ev);
    }
}
