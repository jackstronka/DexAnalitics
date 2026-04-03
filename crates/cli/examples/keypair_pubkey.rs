//! Print base58 public key from a Solana CLI keypair JSON file (64 u8 array).
//!
//! ```text
//! cargo run -p clmm-lp-cli --example keypair_pubkey -- "%USERPROFILE%\.config\solana\clmm_lp_bot_mainnet.json"
//! ```

use solana_sdk::signature::{Keypair, Signer};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: cargo run -p clmm-lp-cli --example keypair_pubkey -- <keypair.json>")?;
    let data = std::fs::read_to_string(&path)?;
    let vec: Vec<u8> = serde_json::from_str(&data)?;
    let arr: [u8; 64] = vec
        .try_into()
        .map_err(|_| "expected a JSON array of exactly 64 bytes")?;
    let secret: [u8; 32] = arr[..32]
        .try_into()
        .map_err(|_| "expected the first 32 bytes to be the secret key")?;
    let kp = Keypair::new_from_array(secret);
    println!("{}", kp.pubkey());
    Ok(())
}
