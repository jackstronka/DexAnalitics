//! Parse Meteora DLMM swap Anchor events from `Program data:` (base64) in transaction logs.
//!
//! Layout matches `carbon_meteora_dlmm_decoder::instructions::swap_event::SwapEvent` (Borsh) and
//! Meteora’s published IDL (`MeteoraAg/dlmm-sdk` `idls/dlmm.json`): the **event is named `Swap`**
//! (not `SwapEvent`). Discriminator = first 8 bytes of `sha256("event:Swap")`.
//!
//! Do **not** confuse with Raydium’s `event:SwapEvent` (`sha256("event:SwapEvent")[..8]` is
//! different from Meteora’s swap payload — Jupiter bundles can contain unrelated `Program data:`
//! lines). See `doc/METEORA_DLMM_SWAP_EVENT.md`.

use borsh::{BorshDeserialize, BorshSerialize};
use solana_sdk::pubkey::Pubkey;

/// `sha256("event:Swap")[..8]` — Meteora DLMM IDL `events[].name == "Swap"`.
pub const METEORA_SWAP_EVENT_DISCRIMINATOR: [u8; 8] = [81, 108, 227, 190, 205, 208, 10, 196];

/// On-chain swap event (Borsh), aligned with Meteora DLMM IDL / carbon decoder.
#[derive(BorshDeserialize, BorshSerialize, Clone, Debug, PartialEq, Eq)]
pub struct MeteoraDlmmSwapEvent {
    pub lb_pair: Pubkey,
    pub from: Pubkey,
    pub start_bin_id: i32,
    pub end_bin_id: i32,
    pub amount_in: u64,
    pub amount_out: u64,
    pub swap_for_y: bool,
    pub fee: u64,
    pub protocol_fee: u64,
    pub fee_bps: u128,
    pub host_fee: u64,
}

const DISCS: [[u8; 8]; 1] = [METEORA_SWAP_EVENT_DISCRIMINATOR];

/// Decode the first swap event in logs whose `lb_pair` matches `pool_address` (LB pair pubkey).
#[must_use]
pub fn parse_meteora_swap_event_for_pool(
    logs: &[String],
    pool_address: &str,
) -> Option<MeteoraDlmmSwapEvent> {
    for line in logs {
        let rest = line
            .strip_prefix("Program data: ")
            .or_else(|| line.strip_prefix("Program Data: "))
            .map(str::trim)?;
        let bytes = base64_decode(rest)?;
        if bytes.len() < 8 + 32 {
            continue;
        }
        for disc in &DISCS {
            if bytes[..8] != *disc {
                continue;
            }
            let mut payload = &bytes[8..];
            let ev = MeteoraDlmmSwapEvent::deserialize(&mut payload).ok()?;
            if ev.lb_pair.to_string() == pool_address {
                return Some(ev);
            }
        }
    }
    None
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(s.trim())
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_synthetic_meteora_swap_event() {
        let pair = Pubkey::new_unique();
        let ev = MeteoraDlmmSwapEvent {
            lb_pair: pair,
            from: Pubkey::new_unique(),
            start_bin_id: -100,
            end_bin_id: 50,
            amount_in: 1_000_000,
            amount_out: 900_000,
            swap_for_y: true,
            fee: 3000,
            protocol_fee: 100,
            fee_bps: 30,
            host_fee: 0,
        };
        let mut buf = METEORA_SWAP_EVENT_DISCRIMINATOR.to_vec();
        buf.extend_from_slice(&borsh::to_vec(&ev).unwrap());
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&buf);
        let line = format!("Program data: {b64}");
        let parsed = parse_meteora_swap_event_for_pool(&[line], &pair.to_string()).expect("parse");
        assert_eq!(parsed, ev);
    }
}
