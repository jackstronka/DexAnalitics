use anyhow::Result;

pub async fn orca_snapshot(pool_address: &str) -> Result<()> {
    use chrono::Utc;
    use rust_decimal::Decimal;
    use rust_decimal::prelude::ToPrimitive;
    use serde::Serialize;
    use solana_sdk::pubkey::Pubkey;
    use spl_token::solana_program::program_pack::Pack;
    use spl_token::state::Account as SplTokenAccount;
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;
    use std::str::FromStr;

    #[derive(Debug, Serialize)]
    struct OrcaWhirlpoolSnapshot {
        ts_utc: String,
        slot: u64,
        pool_address: String,
        token_mint_a: String,
        token_mint_b: String,
        token_vault_a: String,
        token_vault_b: String,
        vault_amount_a: u64,
        vault_amount_b: u64,
        liquidity_active: String,
        tick_current: i32,
        tick_spacing: u16,
        tick_neighborhood: [i32; 5],
        sqrt_price_x64: String,
        fee_rate_raw: u16,
        protocol_fee_rate_bps: u16,
        fee_growth_global_a: String,
        fee_growth_global_b: String,
        protocol_fee_owed_a: u64,
        protocol_fee_owed_b: u64,
        effective_fee_rate_pct: f64,
    }

    let rpc = std::sync::Arc::new(clmm_lp_protocols::rpc::RpcProvider::mainnet());
    let slot = rpc.get_slot().await.unwrap_or(0);
    let reader = clmm_lp_protocols::orca::pool_reader::WhirlpoolReader::new(rpc.clone());
    let state = reader.get_pool_state(pool_address).await?;

    // Fetch vault SPL token accounts and decode balances.
    let va = Pubkey::from_str(&state.token_vault_a.to_string())?;
    let vb = Pubkey::from_str(&state.token_vault_b.to_string())?;
    let accounts = rpc.get_multiple_accounts(&[va, vb]).await?;
    let vault_amount_a = accounts
        .get(0)
        .and_then(|a| a.as_ref())
        .and_then(|a| SplTokenAccount::unpack(&a.data).ok())
        .map(|a| a.amount)
        .unwrap_or(0);
    let vault_amount_b = accounts
        .get(1)
        .and_then(|a| a.as_ref())
        .and_then(|a| SplTokenAccount::unpack(&a.data).ok())
        .map(|a| a.amount)
        .unwrap_or(0);

    let base_fee = state.fee_rate();
    let proto = Decimal::from(state.protocol_fee_rate_bps) / Decimal::from(10_000);
    let eff = clmm_lp_domain::prelude::calculate_effective_fee_rate(base_fee, proto);
    let eff_pct = eff.to_f64().unwrap_or(0.0) * 100.0;

    let snap = OrcaWhirlpoolSnapshot {
        ts_utc: Utc::now().to_rfc3339(),
        slot,
        pool_address: pool_address.to_string(),
        token_mint_a: state.token_mint_a.to_string(),
        token_mint_b: state.token_mint_b.to_string(),
        token_vault_a: state.token_vault_a.to_string(),
        token_vault_b: state.token_vault_b.to_string(),
        vault_amount_a,
        vault_amount_b,
        liquidity_active: state.liquidity.to_string(),
        tick_current: state.tick_current,
        tick_spacing: state.tick_spacing,
        tick_neighborhood: [
            state.tick_current - (state.tick_spacing as i32 * 2),
            state.tick_current - state.tick_spacing as i32,
            state.tick_current,
            state.tick_current + state.tick_spacing as i32,
            state.tick_current + (state.tick_spacing as i32 * 2),
        ],
        sqrt_price_x64: state.sqrt_price.to_string(),
        fee_rate_raw: state.fee_rate_bps,
        protocol_fee_rate_bps: state.protocol_fee_rate_bps,
        fee_growth_global_a: state.fee_growth_global_a.to_string(),
        fee_growth_global_b: state.fee_growth_global_b.to_string(),
        protocol_fee_owed_a: state.protocol_fee_owed_a,
        protocol_fee_owed_b: state.protocol_fee_owed_b,
        effective_fee_rate_pct: eff_pct,
    };

    let mut dir = PathBuf::from("data");
    dir.push("pool-snapshots");
    dir.push("orca");
    dir.push(pool_address);
    fs::create_dir_all(&dir)?;

    let mut path = dir;
    path.push("snapshots.jsonl");

    let line = serde_json::to_string(&snap)? + "\n";
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    f.write_all(line.as_bytes())?;

    println!("✅ Snapshot appended: {}", path.display());
    Ok(())
}

pub async fn orca_snapshot_curated(limit: Option<usize>) -> Result<()> {
    use chrono::Utc;
    use rust_decimal::prelude::ToPrimitive;
    use serde::Serialize;
    use spl_token::solana_program::program_pack::Pack;
    use spl_token::state::Account as SplTokenAccount;

    let startup_path = std::path::Path::new("STARTUP.md");
    let content = std::fs::read_to_string(startup_path).map_err(|e| {
        anyhow::anyhow!(
            "Failed to read STARTUP.md at {}: {}",
            startup_path.display(),
            e
        )
    })?;

    // Very small parser: read "Orca (Whirlpool)" section until next "**Meteora**" or "**Raydium**",
    // then extract addresses inside backticks.
    let mut in_orca_section = false;
    let mut done = false;
    let mut pool_addrs: Vec<String> = Vec::new();

    for line in content.lines() {
        if line.contains("**Orca (Whirlpool)**")
            || line.trim_start().starts_with("**Orca (Whirlpool)**")
        {
            in_orca_section = true;
            continue;
        }
        if in_orca_section && (line.contains("**Meteora**") || line.contains("**Raydium**")) {
            done = true;
        }
        if done {
            break;
        }
        if in_orca_section {
            // Extract text between first pair of backticks in the line.
            let chars = line.chars().collect::<Vec<_>>();
            let is_solana_pubkey = |s: &str| {
                // Solana addresses are base58.
                if s.len() < 32 || s.len() > 44 {
                    return false;
                }
                if s.contains('-') {
                    return false;
                }
                s.chars()
                    .all(|c| matches!(c, '1'..='9' | 'A'..='Z' | 'a'..='z'))
            };

            let mut i = 0usize;
            while i < chars.len() {
                if chars[i] == '`' {
                    // find next backtick
                    if let Some(j) = (i + 1..chars.len()).find(|&k| chars[k] == '`') {
                        let addr: String = chars[i + 1..j].iter().collect();
                        if is_solana_pubkey(&addr) && !pool_addrs.contains(&addr) {
                            pool_addrs.push(addr);
                            if limit.map(|l| pool_addrs.len() >= l).unwrap_or(false) {
                                done = true;
                            }
                        }
                        i = j + 1;
                        continue;
                    }
                }
                i += 1;
            }
        }
    }

    if pool_addrs.is_empty() {
        return Err(anyhow::anyhow!(
            "No Orca pools found in STARTUP.md Orca section"
        ));
    }

    let rpc = std::sync::Arc::new(clmm_lp_protocols::rpc::RpcProvider::mainnet());
    for pool_address in pool_addrs.into_iter() {
        // Reuse the single-pool logic by calling the same internal code path:
        // (duplicated here to avoid restructuring the command handler).
        use clmm_lp_domain::prelude::calculate_effective_fee_rate;
        use rust_decimal::Decimal;

        #[derive(Debug, Serialize)]
        struct OrcaWhirlpoolSnapshot {
            ts_utc: String,
            slot: u64,
            pool_address: String,
            token_mint_a: String,
            token_mint_b: String,
            token_vault_a: String,
            token_vault_b: String,
            vault_amount_a: u64,
            vault_amount_b: u64,
            liquidity_active: String,
            tick_current: i32,
            tick_spacing: u16,
            tick_neighborhood: [i32; 5],
            sqrt_price_x64: String,
            fee_rate_raw: u16,
            protocol_fee_rate_bps: u16,
            fee_growth_global_a: String,
            fee_growth_global_b: String,
            protocol_fee_owed_a: u64,
            protocol_fee_owed_b: u64,
            effective_fee_rate_pct: f64,
        }

        let slot_now = rpc.get_slot().await.unwrap_or(0);
        let reader = clmm_lp_protocols::orca::pool_reader::WhirlpoolReader::new(rpc.clone());
        let state = reader.get_pool_state(&pool_address).await?;

        // `WhirlpoolState` already provides vault addresses as Pubkey.
        let va = state.token_vault_a;
        let vb = state.token_vault_b;
        let accounts = rpc.get_multiple_accounts(&[va, vb]).await?;

        let vault_amount_a = accounts
            .get(0)
            .and_then(|a| a.as_ref())
            .and_then(|a| SplTokenAccount::unpack(&a.data).ok())
            .map(|a| a.amount)
            .unwrap_or(0);
        let vault_amount_b = accounts
            .get(1)
            .and_then(|a| a.as_ref())
            .and_then(|a| SplTokenAccount::unpack(&a.data).ok())
            .map(|a| a.amount)
            .unwrap_or(0);

        let base_fee = state.fee_rate();
        let proto = Decimal::from(state.protocol_fee_rate_bps) / Decimal::from(10_000);
        let eff = calculate_effective_fee_rate(base_fee, proto);
        let eff_pct = eff.to_f64().unwrap_or(0.0) * 100.0;

        let snap = OrcaWhirlpoolSnapshot {
            ts_utc: Utc::now().to_rfc3339(),
            slot: slot_now,
            pool_address: pool_address.to_string(),
            token_mint_a: state.token_mint_a.to_string(),
            token_mint_b: state.token_mint_b.to_string(),
            token_vault_a: state.token_vault_a.to_string(),
            token_vault_b: state.token_vault_b.to_string(),
            vault_amount_a,
            vault_amount_b,
            liquidity_active: state.liquidity.to_string(),
            tick_current: state.tick_current,
            tick_spacing: state.tick_spacing,
            tick_neighborhood: [
                state.tick_current - (state.tick_spacing as i32 * 2),
                state.tick_current - state.tick_spacing as i32,
                state.tick_current,
                state.tick_current + state.tick_spacing as i32,
                state.tick_current + (state.tick_spacing as i32 * 2),
            ],
            sqrt_price_x64: state.sqrt_price.to_string(),
            fee_rate_raw: state.fee_rate_bps,
            protocol_fee_rate_bps: state.protocol_fee_rate_bps,
            fee_growth_global_a: state.fee_growth_global_a.to_string(),
            fee_growth_global_b: state.fee_growth_global_b.to_string(),
            protocol_fee_owed_a: state.protocol_fee_owed_a,
            protocol_fee_owed_b: state.protocol_fee_owed_b,
            effective_fee_rate_pct: eff_pct,
        };

        let mut dir = std::path::PathBuf::from("data");
        dir.push("pool-snapshots");
        dir.push("orca");
        dir.push(&pool_address);
        std::fs::create_dir_all(&dir)?;

        let mut path = dir;
        path.push("snapshots.jsonl");

        let line = serde_json::to_string(&snap)? + "\n";
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        use std::io::Write;
        f.write_all(line.as_bytes())?;

        println!("✅ Snapshot appended: {}", path.display());
    }
    Ok(())
}

pub async fn raydium_snapshot_curated(limit: Option<usize>) -> Result<()> {
    use base64::{Engine, engine::general_purpose::STANDARD as BASE64_STANDARD};
    use spl_token::solana_program::program_pack::Pack;
    use spl_token::state::Account as SplTokenAccount;

    let startup_path = std::path::Path::new("STARTUP.md");
    let content = std::fs::read_to_string(startup_path).map_err(|e| {
        anyhow::anyhow!(
            "Failed to read STARTUP.md at {}: {}",
            startup_path.display(),
            e
        )
    })?;

    let mut in_section = false;
    let mut done = false;
    let mut pool_addrs: Vec<String> = Vec::new();

    let is_solana_pubkey = |s: &str| {
        if s.len() < 32 || s.len() > 44 {
            return false;
        }
        if s.contains('-') {
            return false;
        }
        s.chars()
            .all(|c| matches!(c, '1'..='9' | 'A'..='Z' | 'a'..='z'))
    };

    for line in content.lines() {
        if line.trim_start().starts_with("**Raydium**") {
            in_section = true;
            continue;
        }
        // Raydium section ends before the numbered steps.
        if in_section && line.trim_start().starts_with("1.") {
            done = true;
        }
        if done {
            break;
        }
        if in_section {
            let chars = line.chars().collect::<Vec<_>>();
            let mut i = 0usize;
            while i < chars.len() {
                if chars[i] == '`' {
                    if let Some(j) = (i + 1..chars.len()).find(|&k| chars[k] == '`') {
                        let addr: String = chars[i + 1..j].iter().collect();
                        if is_solana_pubkey(&addr) && !pool_addrs.contains(&addr) {
                            pool_addrs.push(addr.clone());
                            if limit.map(|l| pool_addrs.len() >= l).unwrap_or(false) {
                                done = true;
                            }
                        }
                        i = j + 1;
                        continue;
                    }
                }
                i += 1;
            }
        }
    }

    if pool_addrs.is_empty() {
        return Err(anyhow::anyhow!(
            "No Raydium pools found in STARTUP.md Raydium section"
        ));
    }

    println!(
        "RaydiumSnapshotCurated: selected pools={} limit={:?}",
        pool_addrs.len(),
        limit
    );
    let rpc = std::sync::Arc::new(clmm_lp_protocols::rpc::RpcProvider::mainnet());
    println!("RaydiumSnapshotCurated: fetching current RPC slot...");
    let slot_now = rpc.get_slot().await.unwrap_or(0);
    println!("RaydiumSnapshotCurated: slot_now={}", slot_now);

    #[derive(Debug, serde::Serialize)]
    struct RaydiumClmmSnapshot {
        ts_utc: String,
        slot: u64,
        protocol: String,
        pool_address: String,
        owner: String,
        lamports: u64,

        // Keep the raw payload for debugging / future decoding improvements.
        data_len: usize,
        data_b64: String,

        /// Whether the Raydium pool account bytes were decoded successfully.
        parse_ok: bool,
        /// Detailed decode error (only set when `parse_ok=false`).
        #[serde(skip_serializing_if = "Option::is_none")]
        parse_error: Option<String>,

        #[serde(skip_serializing_if = "Option::is_none")]
        token_mint_a: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        token_mint_b: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        token_vault_a: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        token_vault_b: Option<String>,

        #[serde(skip_serializing_if = "Option::is_none")]
        vault_amount_a: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        vault_amount_b: Option<u64>,

        #[serde(skip_serializing_if = "Option::is_none")]
        mint_decimals_a: Option<u8>,
        #[serde(skip_serializing_if = "Option::is_none")]
        mint_decimals_b: Option<u8>,

        #[serde(skip_serializing_if = "Option::is_none")]
        liquidity_active: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tick_current: Option<i32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tick_neighborhood: Option<[i32; 5]>,
        #[serde(skip_serializing_if = "Option::is_none")]
        sqrt_price_x64: Option<String>,

        #[serde(skip_serializing_if = "Option::is_none")]
        fee_growth_global_a_x64: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        fee_growth_global_b_x64: Option<String>,

        #[serde(skip_serializing_if = "Option::is_none")]
        protocol_fees_token_a: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        protocol_fees_token_b: Option<u64>,
    }

    for pool_address in pool_addrs.into_iter() {
        println!("RaydiumSnapshotCurated: fetching account {}", pool_address);
        let acct = rpc.get_account_by_address(&pool_address).await?;
        let (parsed, parse_ok, parse_error) =
            match clmm_lp_protocols::raydium::pool_reader::parse_pool_state(&acct.data) {
                Ok(p) => (Some(p), true, None),
                Err(e) => (None, false, Some(e.to_string())),
            };

        let (vault_amount_a, vault_amount_b) = if let Some(ref p) = parsed {
            use std::str::FromStr;
            let va = solana_sdk::pubkey::Pubkey::from_str(&p.token_vault0).ok();
            let vb = solana_sdk::pubkey::Pubkey::from_str(&p.token_vault1).ok();
            if let (Some(va), Some(vb)) = (va, vb) {
                match rpc.get_multiple_accounts(&[va, vb]).await {
                    Ok(accounts) => {
                        let a = accounts
                            .get(0)
                            .and_then(|a| a.as_ref())
                            .and_then(|a| SplTokenAccount::unpack(&a.data).ok())
                            .map(|a| a.amount);
                        let b = accounts
                            .get(1)
                            .and_then(|a| a.as_ref())
                            .and_then(|a| SplTokenAccount::unpack(&a.data).ok())
                            .map(|a| a.amount);
                        (a, b)
                    }
                    Err(_) => (None, None),
                }
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };

        let snap = RaydiumClmmSnapshot {
            ts_utc: chrono::Utc::now().to_rfc3339(),
            slot: slot_now,
            protocol: "raydium".to_string(),
            pool_address: pool_address.clone(),
            owner: acct.owner.to_string(),
            lamports: acct.lamports,
            data_len: acct.data.len(),
            data_b64: BASE64_STANDARD.encode(&acct.data),
            parse_ok,
            parse_error,
            token_mint_a: parsed.as_ref().map(|p| p.token_mint0.to_string()),
            token_mint_b: parsed.as_ref().map(|p| p.token_mint1.to_string()),
            token_vault_a: parsed.as_ref().map(|p| p.token_vault0.to_string()),
            token_vault_b: parsed.as_ref().map(|p| p.token_vault1.to_string()),
            vault_amount_a,
            vault_amount_b,
            mint_decimals_a: parsed.as_ref().map(|p| p.mint_decimals0),
            mint_decimals_b: parsed.as_ref().map(|p| p.mint_decimals1),
            liquidity_active: parsed.as_ref().map(|p| p.liquidity_active.to_string()),
            tick_current: parsed.as_ref().map(|p| p.tick_current),
            tick_neighborhood: parsed.as_ref().map(|p| {
                [
                    p.tick_current - 100,
                    p.tick_current - 10,
                    p.tick_current,
                    p.tick_current + 10,
                    p.tick_current + 100,
                ]
            }),
            sqrt_price_x64: parsed.as_ref().map(|p| p.sqrt_price_x64.to_string()),
            fee_growth_global_a_x64: parsed
                .as_ref()
                .map(|p| p.fee_growth_global0_x64.to_string()),
            fee_growth_global_b_x64: parsed
                .as_ref()
                .map(|p| p.fee_growth_global1_x64.to_string()),
            protocol_fees_token_a: parsed.as_ref().map(|p| p.protocol_fees_token0),
            protocol_fees_token_b: parsed.as_ref().map(|p| p.protocol_fees_token1),
        };

        let mut dir = std::path::PathBuf::from("data");
        dir.push("pool-snapshots");
        dir.push("raydium");
        dir.push(&pool_address);
        std::fs::create_dir_all(&dir)?;

        let mut path = dir;
        path.push("snapshots.jsonl");

        let line = serde_json::to_string(&snap)?;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        use std::io::Write;
        f.write_all(line.as_bytes())?;
        f.write_all(b"\n")?;

        println!("✅ Snapshot appended: {}", path.display());
    }

    Ok(())
}

pub async fn meteora_snapshot_curated(limit: Option<usize>) -> Result<()> {
    use base64::{Engine, engine::general_purpose::STANDARD as BASE64_STANDARD};
    use spl_token::solana_program::program_pack::Pack;
    use spl_token::state::Account as SplTokenAccount;

    let startup_path = std::path::Path::new("STARTUP.md");
    let content = std::fs::read_to_string(startup_path).map_err(|e| {
        anyhow::anyhow!(
            "Failed to read STARTUP.md at {}: {}",
            startup_path.display(),
            e
        )
    })?;

    let mut in_section = false;
    let mut done = false;
    let mut pool_addrs: Vec<String> = Vec::new();

    let is_solana_pubkey = |s: &str| {
        if s.len() < 32 || s.len() > 44 {
            return false;
        }
        if s.contains('-') {
            return false;
        }
        s.chars()
            .all(|c| matches!(c, '1'..='9' | 'A'..='Z' | 'a'..='z'))
    };

    for line in content.lines() {
        if line.trim_start().starts_with("**Meteora**") {
            in_section = true;
            continue;
        }
        // Meteora section ends before the Raydium section.
        if in_section && line.contains("**Raydium**") {
            done = true;
        }
        if done {
            break;
        }
        if in_section {
            let chars = line.chars().collect::<Vec<_>>();
            let mut i = 0usize;
            while i < chars.len() {
                if chars[i] == '`' {
                    if let Some(j) = (i + 1..chars.len()).find(|&k| chars[k] == '`') {
                        let addr: String = chars[i + 1..j].iter().collect();
                        if is_solana_pubkey(&addr) && !pool_addrs.contains(&addr) {
                            pool_addrs.push(addr.clone());
                            if limit.map(|l| pool_addrs.len() >= l).unwrap_or(false) {
                                done = true;
                            }
                        }
                        i = j + 1;
                        continue;
                    }
                }
                i += 1;
            }
        }
    }

    if pool_addrs.is_empty() {
        return Err(anyhow::anyhow!(
            "No Meteora pools found in STARTUP.md Meteora section"
        ));
    }

    let rpc = std::sync::Arc::new(clmm_lp_protocols::rpc::RpcProvider::mainnet());
    let slot_now = rpc.get_slot().await.unwrap_or(0);

    #[derive(Debug, serde::Serialize)]
    struct MeteoraLbPairSnapshot {
        ts_utc: String,
        slot: u64,
        protocol: String,
        pool_address: String,
        owner: String,
        lamports: u64,

        // Keep raw payload for later deeper decoding (bin arrays, fee growth, etc.).
        data_len: usize,
        data_b64: String,

        /// Whether the Meteora lb_pair account bytes were decoded successfully.
        parse_ok: bool,
        /// Detailed decode error (only set when `parse_ok=false`).
        #[serde(skip_serializing_if = "Option::is_none")]
        parse_error: Option<String>,

        #[serde(skip_serializing_if = "Option::is_none")]
        active_id: Option<i32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        active_bin_neighborhood: Option<[i32; 5]>,
        #[serde(skip_serializing_if = "Option::is_none")]
        bin_step: Option<u16>,

        #[serde(skip_serializing_if = "Option::is_none")]
        token_mint_a: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        token_mint_b: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        token_vault_a: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        token_vault_b: Option<String>,

        #[serde(skip_serializing_if = "Option::is_none")]
        token_vault_owner_a: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        token_vault_owner_b: Option<String>,

        #[serde(skip_serializing_if = "Option::is_none")]
        vault_amount_a: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        vault_amount_b: Option<u64>,

        #[serde(skip_serializing_if = "Option::is_none")]
        protocol_fee_amount_a: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        protocol_fee_amount_b: Option<u64>,
    }

    for pool_address in pool_addrs.into_iter() {
        let acct = rpc.get_account_by_address(&pool_address).await?;
        let (parsed, parse_ok, parse_error) =
            match clmm_lp_protocols::meteora::pool_reader::parse_lb_pair(&acct.data) {
                Ok(p) => (Some(p), true, None),
                Err(e) => (None, false, Some(e.to_string())),
            };

        let (vault_amount_a, token_vault_owner_a, vault_amount_b, token_vault_owner_b) =
            if let Some(ref p) = parsed {
                // Try bulk first (cheaper RPC), but fall back to individual fetches
                // if the bulk request was partially/fully missing.
                let accounts_bulk = rpc
                    .get_multiple_accounts(&[p.reserve_x, p.reserve_y])
                    .await
                    .ok();

                let unpack_a = accounts_bulk.as_ref().and_then(|accounts| {
                    accounts
                        .get(0)
                        .and_then(|a| a.as_ref())
                        .and_then(|a| SplTokenAccount::unpack(&a.data).ok())
                });
                let unpack_b = accounts_bulk.as_ref().and_then(|accounts| {
                    accounts
                        .get(1)
                        .and_then(|a| a.as_ref())
                        .and_then(|a| SplTokenAccount::unpack(&a.data).ok())
                });

                let a = if unpack_a.is_some() {
                    unpack_a
                } else {
                    rpc.get_account_by_address(&p.reserve_x.to_string())
                        .await
                        .ok()
                        .and_then(|acc| SplTokenAccount::unpack(&acc.data).ok())
                };
                let b = if unpack_b.is_some() {
                    unpack_b
                } else {
                    rpc.get_account_by_address(&p.reserve_y.to_string())
                        .await
                        .ok()
                        .and_then(|acc| SplTokenAccount::unpack(&acc.data).ok())
                };

                (
                    a.as_ref().map(|a| a.amount),
                    a.as_ref().map(|a| a.owner.to_string()),
                    b.as_ref().map(|b| b.amount),
                    b.as_ref().map(|b| b.owner.to_string()),
                )
            } else {
                (None, None, None, None)
            };

        let snap = MeteoraLbPairSnapshot {
            ts_utc: chrono::Utc::now().to_rfc3339(),
            slot: slot_now,
            protocol: "meteora".to_string(),
            pool_address: pool_address.clone(),
            owner: acct.owner.to_string(),
            lamports: acct.lamports,
            data_len: acct.data.len(),
            data_b64: BASE64_STANDARD.encode(&acct.data),
            parse_ok,
            parse_error,
            active_id: parsed.as_ref().map(|p| p.active_id),
            active_bin_neighborhood: parsed.as_ref().map(|p| {
                [
                    p.active_id - 2,
                    p.active_id - 1,
                    p.active_id,
                    p.active_id + 1,
                    p.active_id + 2,
                ]
            }),
            bin_step: parsed.as_ref().map(|p| p.bin_step),
            token_mint_a: parsed.as_ref().map(|p| p.token_mint_x.to_string()),
            token_mint_b: parsed.as_ref().map(|p| p.token_mint_y.to_string()),
            token_vault_a: parsed.as_ref().map(|p| p.reserve_x.to_string()),
            token_vault_b: parsed.as_ref().map(|p| p.reserve_y.to_string()),
            token_vault_owner_a,
            token_vault_owner_b,
            vault_amount_a,
            vault_amount_b,
            protocol_fee_amount_a: parsed.as_ref().map(|p| p.protocol_fee_amount_x),
            protocol_fee_amount_b: parsed.as_ref().map(|p| p.protocol_fee_amount_y),
        };

        let mut dir = std::path::PathBuf::from("data");
        dir.push("pool-snapshots");
        dir.push("meteora");
        dir.push(&pool_address);
        std::fs::create_dir_all(&dir)?;

        let mut path = dir;
        path.push("snapshots.jsonl");

        let line = serde_json::to_string(&snap)?;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        use std::io::Write;
        f.write_all(line.as_bytes())?;
        f.write_all(b"\n")?;

        println!("✅ Snapshot appended: {}", path.display());
    }

    Ok(())
}
