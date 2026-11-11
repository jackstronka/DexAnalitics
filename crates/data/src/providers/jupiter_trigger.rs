//! Jupiter Trigger Order API V2 — auth + vault/deposit + price + DCA (E0–E5).
//!
//! Docs: <https://developers.jup.ag/docs/trigger>
//! Live writes gated by Dex API (`mode: live` + `confirm_execute`; DCA also needs `dca_allowed`).

use anyhow::{Result, anyhow};
use reqwest::Client;
use serde::{Deserialize, Serialize};

const DEFAULT_BASE: &str = "https://api.jup.ag/trigger/v2";

/// JWT session for Trigger authenticated calls.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TriggerAuthTokens {
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    pub wallet_pubkey: String,
}

/// Challenge returned by `POST /auth/challenge`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum TriggerChallenge {
    /// Wallet signs this message (standard wallets).
    Message { challenge: String },
    /// Hardware wallets: sign this serialized transaction.
    Transaction { transaction: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TriggerOrderHistoryItem {
    pub id: String,
    #[serde(default)]
    pub order_type: Option<String>,
    #[serde(default)]
    pub order_state: Option<String>,
    #[serde(default)]
    pub input_mint: Option<String>,
    #[serde(default)]
    pub output_mint: Option<String>,
    #[serde(default)]
    pub trigger_mint: Option<String>,
    #[serde(default)]
    pub trigger_condition: Option<String>,
    #[serde(default)]
    pub trigger_price_usd: Option<f64>,
    #[serde(default)]
    pub initial_input_amount: Option<String>,
    #[serde(default)]
    pub remaining_input_amount: Option<String>,
    #[serde(default)]
    pub expires_at: Option<i64>,
    #[serde(default)]
    pub created_at: Option<i64>,
    #[serde(default)]
    pub updated_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TriggerOrderPagination {
    pub total: u64,
    pub limit: u64,
    pub offset: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TriggerOrderHistoryResponse {
    pub orders: Vec<TriggerOrderHistoryItem>,
    pub pagination: TriggerOrderPagination,
}

#[derive(Debug, Clone, Default)]
pub struct OrderHistoryQuery {
    pub state: Option<String>,
    pub mint: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

/// `GET /vault` / `GET /vault/register` response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TriggerVault {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_pubkey: Option<String>,
    pub vault_pubkey: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub privy_vault_id: Option<String>,
}

/// Request body for `POST /deposit/craft`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TriggerDepositCraftRequest {
    pub input_mint: String,
    pub output_mint: String,
    pub user_address: String,
    /// Amount in input mint smallest units.
    pub amount: String,
    /// `price` or `dca`.
    pub order_type: String,
    /// Required for `price`: `single` | `oco` | `otoco`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order_sub_type: Option<String>,
}

/// Response from `POST /deposit/craft` (unsigned tx — sign client/host side).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TriggerDepositCraftResponse {
    pub transaction: String,
    pub request_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receiver_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_decimals: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_token_account: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_token_account: Option<String>,
}

/// `POST /orders/price` body for a **single** price order (Buy-the-Dip / TP / SL).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TriggerPriceOrderCreateRequest {
    /// Create variant: `single` (OCO/OTOCO out of scope for E2).
    pub order_type: String,
    pub deposit_request_id: String,
    pub deposit_signed_tx: String,
    pub user_pubkey: String,
    pub input_mint: String,
    pub output_mint: String,
    pub input_amount: String,
    pub trigger_mint: String,
    pub trigger_condition: String,
    pub trigger_price_usd: f64,
    pub expires_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slippage_bps: Option<u32>,
}

/// Response from `POST /orders/price`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TriggerPriceOrderCreateResponse {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx_signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deposit_confirmed: Option<bool>,
}

/// Response from `POST /orders/price/cancel/{id}` (unsigned withdraw tx).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TriggerPriceOrderCancelResponse {
    pub id: String,
    /// Base64 unsigned withdraw transaction.
    pub transaction: String,
    pub request_id: String,
}

/// Body for `POST /orders/price/confirm-cancel/{id}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TriggerPriceOrderConfirmCancelRequest {
    pub signed_transaction: String,
    pub cancel_request_id: String,
}

/// Response from `POST /orders/price/confirm-cancel/{id}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TriggerPriceOrderConfirmCancelResponse {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx_signature: Option<String>,
}

/// `PATCH /orders/price/{id}` body for a **single** order (T14).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TriggerPriceOrderPatchRequest {
    /// Must be `single` for E4 (OCO/OTOCO/trailing out of scope).
    pub order_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_price_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slippage_bps: Option<u32>,
}

/// Response from `PATCH /orders/price/{id}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TriggerPriceOrderPatchResponse {
    pub id: String,
}

/// `POST /orders/dca` body (time_based; price_conditional fields optional for later).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TriggerDcaOrderCreateRequest {
    pub deposit_request_id: String,
    pub deposit_signed_tx: String,
    pub user_pubkey: String,
    pub input_mint: String,
    pub output_mint: String,
    pub input_amount: String,
    pub order_count: u32,
    pub interval_seconds: u64,
    /// `time_based` (E5) or `price_conditional`.
    #[serde(default = "default_dca_order_type")]
    pub order_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_price_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_price_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_mint: Option<String>,
}

fn default_dca_order_type() -> String {
    "time_based".into()
}

/// Response from `POST /orders/dca` / confirm-cancel DCA.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TriggerDcaOrderTxResponse {
    pub id: String,
    pub tx_signature: String,
}

/// Item from `GET /orders/history/dca`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TriggerDcaHistoryItem {
    pub id: String,
    #[serde(default)]
    pub order_state: Option<String>,
    #[serde(default)]
    pub order_type: Option<String>,
    #[serde(default)]
    pub input_mint: Option<String>,
    #[serde(default)]
    pub output_mint: Option<String>,
    #[serde(default)]
    pub input_amount: Option<String>,
    #[serde(default)]
    pub order_count: Option<u32>,
    #[serde(default)]
    pub interval_seconds: Option<u64>,
    #[serde(default)]
    pub rounds_remaining: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TriggerDcaHistoryResponse {
    pub orders: Vec<TriggerDcaHistoryItem>,
    #[serde(default)]
    pub pagination: Option<TriggerOrderPagination>,
}

/// Response from `POST /orders/dca/cancel/{id}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TriggerDcaOrderCancelResponse {
    pub id: String,
    pub transaction: String,
    pub request_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rounds_remaining: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refund_amount: Option<String>,
}

/// HTTP client for Jupiter Trigger V2 (read + auth helpers).
pub struct JupiterTriggerClient {
    client: Client,
    api_key: Option<String>,
    base_url: String,
}

impl JupiterTriggerClient {
    #[must_use]
    pub fn new(api_key: Option<String>) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url: DEFAULT_BASE.to_string(),
        }
    }

    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    fn require_api_key(&self) -> Result<&str> {
        self.api_key
            .as_deref()
            .filter(|k| !k.is_empty())
            .ok_or_else(|| anyhow!("JUPITER_API_KEY missing — set env or pass api key"))
    }

    fn apply_api_key(&self, mut req: reqwest::RequestBuilder) -> Result<reqwest::RequestBuilder> {
        let key = self.require_api_key()?;
        req = req.header("x-api-key", key);
        Ok(req)
    }

    /// `POST /auth/challenge`
    pub async fn request_challenge(
        &self,
        wallet_pubkey: &str,
        challenge_type: &str,
    ) -> Result<TriggerChallenge> {
        let url = format!("{}/auth/challenge", self.base_url.trim_end_matches('/'));
        let body = serde_json::json!({
            "walletPubkey": wallet_pubkey,
            "type": challenge_type,
        });
        let req = self
            .apply_api_key(self.client.post(&url))?
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json(&body);
        let res = req.send().await?;
        let status = res.status();
        let text = res.text().await?;
        if !status.is_success() {
            return Err(anyhow!("auth/challenge {status}: {text}"));
        }
        Ok(serde_json::from_str(&text)?)
    }

    /// `POST /auth/verify` — prefer `authMode: access_refresh`.
    pub async fn verify_challenge(
        &self,
        wallet_pubkey: &str,
        signature_base58: &str,
        challenge_type: &str,
    ) -> Result<TriggerAuthTokens> {
        let url = format!("{}/auth/verify", self.base_url.trim_end_matches('/'));
        let body = serde_json::json!({
            "type": challenge_type,
            "walletPubkey": wallet_pubkey,
            "signature": signature_base58,
            "authMode": "access_refresh",
        });
        let req = self
            .apply_api_key(self.client.post(&url))?
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json(&body);
        let res = req.send().await?;
        let status = res.status();
        let text = res.text().await?;
        if !status.is_success() {
            return Err(anyhow!("auth/verify {status}: {text}"));
        }
        let json: serde_json::Value = serde_json::from_str(&text)?;
        let access = json
            .get("accessToken")
            .or_else(|| json.get("token"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("verify response missing accessToken/token"))?
            .to_string();
        Ok(TriggerAuthTokens {
            access_token: access,
            refresh_token: json
                .get("refreshToken")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            expires_at: json
                .get("expiresAt")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            wallet_pubkey: wallet_pubkey.to_string(),
        })
    }

    /// `POST /auth/refresh`
    pub async fn refresh(
        &self,
        refresh_token: &str,
        wallet_pubkey: &str,
    ) -> Result<TriggerAuthTokens> {
        let url = format!("{}/auth/refresh", self.base_url.trim_end_matches('/'));
        let body = serde_json::json!({ "refreshToken": refresh_token });
        let req = self
            .apply_api_key(self.client.post(&url))?
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json(&body);
        let res = req.send().await?;
        let status = res.status();
        let text = res.text().await?;
        if !status.is_success() {
            return Err(anyhow!("auth/refresh {status}: {text}"));
        }
        let json: serde_json::Value = serde_json::from_str(&text)?;
        let access = json
            .get("accessToken")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("refresh missing accessToken"))?
            .to_string();
        Ok(TriggerAuthTokens {
            access_token: access,
            refresh_token: json
                .get("refreshToken")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .or_else(|| Some(refresh_token.to_string())),
            expires_at: json
                .get("expiresAt")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            wallet_pubkey: wallet_pubkey.to_string(),
        })
    }

    /// `GET /orders/history`
    pub async fn get_order_history(
        &self,
        access_token: &str,
        query: &OrderHistoryQuery,
    ) -> Result<TriggerOrderHistoryResponse> {
        let mut url = reqwest::Url::parse(&format!(
            "{}/orders/history",
            self.base_url.trim_end_matches('/')
        ))?;
        {
            let mut q = url.query_pairs_mut();
            if let Some(state) = &query.state {
                q.append_pair("state", state);
            }
            if let Some(mint) = &query.mint {
                q.append_pair("mint", mint);
            }
            if let Some(limit) = query.limit {
                q.append_pair("limit", &limit.to_string());
            }
            if let Some(offset) = query.offset {
                q.append_pair("offset", &offset.to_string());
            }
            q.append_pair("sort", "updated_at");
            q.append_pair("dir", "desc");
        }

        let req = self
            .apply_api_key(self.client.get(url))?
            .header("Accept", "application/json")
            .header("Authorization", format!("Bearer {access_token}"));
        let res = req.send().await?;
        let status = res.status();
        let text = res.text().await?;
        if !status.is_success() {
            return Err(anyhow!("orders/history {status}: {text}"));
        }
        Ok(serde_json::from_str(&text)?)
    }

    /// Open Buy-the-Dip style orders: active + `triggerCondition=below` + open/pending.
    pub async fn list_open_buy_dips(
        &self,
        access_token: &str,
        limit: u32,
    ) -> Result<Vec<TriggerOrderHistoryItem>> {
        let hist = self
            .get_order_history(
                access_token,
                &OrderHistoryQuery {
                    state: Some("active".into()),
                    limit: Some(limit),
                    ..Default::default()
                },
            )
            .await?;
        Ok(filter_open_buy_dips(&hist.orders))
    }

    /// `GET /vault`
    pub async fn get_vault(&self, access_token: &str) -> Result<TriggerVault> {
        let req = self
            .apply_api_key(
                self.client
                    .get(format!("{}/vault", self.base_url.trim_end_matches('/'))),
            )?
            .header("Accept", "application/json")
            .header("Authorization", format!("Bearer {access_token}"));
        let res = req.send().await?;
        let status = res.status();
        let text = res.text().await?;
        if !status.is_success() {
            return Err(anyhow!("vault {status}: {text}"));
        }
        Ok(serde_json::from_str(&text)?)
    }

    /// `GET /vault/register` — first-time vault; may 409 if already exists.
    pub async fn register_vault(&self, access_token: &str) -> Result<TriggerVault> {
        let req = self
            .apply_api_key(self.client.get(format!(
                "{}/vault/register",
                self.base_url.trim_end_matches('/')
            )))?
            .header("Accept", "application/json")
            .header("Authorization", format!("Bearer {access_token}"));
        let res = req.send().await?;
        let status = res.status();
        let text = res.text().await?;
        if !status.is_success() {
            return Err(anyhow!("vault/register {status}: {text}"));
        }
        Ok(serde_json::from_str(&text)?)
    }

    /// Resolve vault: `GET /vault`, optionally `GET /vault/register` on failure when `register_if_missing`.
    pub async fn resolve_vault(
        &self,
        access_token: &str,
        register_if_missing: bool,
    ) -> Result<TriggerVault> {
        match self.get_vault(access_token).await {
            Ok(v) => Ok(v),
            Err(e) if register_if_missing => self
                .register_vault(access_token)
                .await
                .map_err(|reg| anyhow!("vault failed ({e}); register failed: {reg}")),
            Err(e) => Err(e),
        }
    }

    /// `POST /deposit/craft` — unsigned deposit tx + `requestId` (no create order).
    pub async fn craft_deposit(
        &self,
        access_token: &str,
        body: &TriggerDepositCraftRequest,
    ) -> Result<TriggerDepositCraftResponse> {
        let url = format!("{}/deposit/craft", self.base_url.trim_end_matches('/'));
        let req = self
            .apply_api_key(self.client.post(url))?
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {access_token}"))
            .json(body);
        let res = req.send().await?;
        let status = res.status();
        let text = res.text().await?;
        if !status.is_success() {
            return Err(anyhow!("deposit/craft {status}: {text}"));
        }
        Ok(serde_json::from_str(&text)?)
    }

    /// `POST /orders/price` — create single price order (consumes signed deposit).
    pub async fn create_price_order(
        &self,
        access_token: &str,
        body: &TriggerPriceOrderCreateRequest,
    ) -> Result<TriggerPriceOrderCreateResponse> {
        let url = format!("{}/orders/price", self.base_url.trim_end_matches('/'));
        let req = self
            .apply_api_key(self.client.post(url))?
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {access_token}"))
            .json(body);
        let res = req.send().await?;
        let status = res.status();
        let text = res.text().await?;
        if !status.is_success() {
            return Err(anyhow!("orders/price {status}: {text}"));
        }
        Ok(serde_json::from_str(&text)?)
    }

    /// `POST /orders/price/cancel/{id}` — start cancel; returns unsigned withdraw tx.
    pub async fn cancel_price_order(
        &self,
        access_token: &str,
        order_id: &str,
    ) -> Result<TriggerPriceOrderCancelResponse> {
        let id = order_id.trim();
        if id.is_empty() {
            return Err(anyhow!("order_id required"));
        }
        let url = format!(
            "{}/orders/price/cancel/{}",
            self.base_url.trim_end_matches('/'),
            id
        );
        let req = self
            .apply_api_key(self.client.post(url))?
            .header("Accept", "application/json")
            .header("Authorization", format!("Bearer {access_token}"));
        let res = req.send().await?;
        let status = res.status();
        let text = res.text().await?;
        if !status.is_success() {
            return Err(anyhow!("orders/price/cancel {status}: {text}"));
        }
        Ok(serde_json::from_str(&text)?)
    }

    /// `POST /orders/price/confirm-cancel/{id}` — submit signed withdraw.
    pub async fn confirm_cancel_price_order(
        &self,
        access_token: &str,
        order_id: &str,
        body: &TriggerPriceOrderConfirmCancelRequest,
    ) -> Result<TriggerPriceOrderConfirmCancelResponse> {
        let id = order_id.trim();
        if id.is_empty() {
            return Err(anyhow!("order_id required"));
        }
        let url = format!(
            "{}/orders/price/confirm-cancel/{}",
            self.base_url.trim_end_matches('/'),
            id
        );
        let req = self
            .apply_api_key(self.client.post(url))?
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {access_token}"))
            .json(body);
        let res = req.send().await?;
        let status = res.status();
        let text = res.text().await?;
        if !status.is_success() {
            return Err(anyhow!("orders/price/confirm-cancel {status}: {text}"));
        }
        Ok(serde_json::from_str(&text)?)
    }

    /// `PATCH /orders/price/{id}` — update trigger price / slippage (single).
    pub async fn patch_price_order(
        &self,
        access_token: &str,
        order_id: &str,
        body: &TriggerPriceOrderPatchRequest,
    ) -> Result<TriggerPriceOrderPatchResponse> {
        let id = order_id.trim();
        if id.is_empty() {
            return Err(anyhow!("order_id required"));
        }
        let url = format!(
            "{}/orders/price/{}",
            self.base_url.trim_end_matches('/'),
            id
        );
        let req = self
            .apply_api_key(self.client.patch(url))?
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {access_token}"))
            .json(body);
        let res = req.send().await?;
        let status = res.status();
        let text = res.text().await?;
        if !status.is_success() {
            return Err(anyhow!("orders/price PATCH {status}: {text}"));
        }
        Ok(serde_json::from_str(&text)?)
    }

    /// `GET /orders/history/dca`
    pub async fn get_dca_order_history(
        &self,
        access_token: &str,
        query: &OrderHistoryQuery,
    ) -> Result<TriggerDcaHistoryResponse> {
        let mut url = format!("{}/orders/history/dca", self.base_url.trim_end_matches('/'));
        let mut params = Vec::new();
        if let Some(s) = &query.state {
            params.push(format!("state={s}"));
        }
        if let Some(m) = &query.mint {
            params.push(format!("mint={m}"));
        }
        if let Some(l) = query.limit {
            params.push(format!("limit={l}"));
        }
        if let Some(o) = query.offset {
            params.push(format!("offset={o}"));
        }
        if !params.is_empty() {
            url.push('?');
            url.push_str(&params.join("&"));
        }
        let req = self
            .apply_api_key(self.client.get(url))?
            .header("Accept", "application/json")
            .header("Authorization", format!("Bearer {access_token}"));
        let res = req.send().await?;
        let status = res.status();
        let text = res.text().await?;
        if !status.is_success() {
            return Err(anyhow!("orders/history/dca {status}: {text}"));
        }
        Ok(serde_json::from_str(&text)?)
    }

    /// `POST /orders/dca` — create DCA (consumes signed deposit crafted with orderType=dca).
    pub async fn create_dca_order(
        &self,
        access_token: &str,
        body: &TriggerDcaOrderCreateRequest,
    ) -> Result<TriggerDcaOrderTxResponse> {
        let url = format!("{}/orders/dca", self.base_url.trim_end_matches('/'));
        let req = self
            .apply_api_key(self.client.post(url))?
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {access_token}"))
            .json(body);
        let res = req.send().await?;
        let status = res.status();
        let text = res.text().await?;
        if !status.is_success() {
            return Err(anyhow!("orders/dca {status}: {text}"));
        }
        Ok(serde_json::from_str(&text)?)
    }

    /// `POST /orders/dca/cancel/{id}`
    pub async fn cancel_dca_order(
        &self,
        access_token: &str,
        order_id: &str,
    ) -> Result<TriggerDcaOrderCancelResponse> {
        let id = order_id.trim();
        if id.is_empty() {
            return Err(anyhow!("order_id required"));
        }
        let url = format!(
            "{}/orders/dca/cancel/{}",
            self.base_url.trim_end_matches('/'),
            id
        );
        let req = self
            .apply_api_key(self.client.post(url))?
            .header("Accept", "application/json")
            .header("Authorization", format!("Bearer {access_token}"));
        let res = req.send().await?;
        let status = res.status();
        let text = res.text().await?;
        if !status.is_success() {
            return Err(anyhow!("orders/dca/cancel {status}: {text}"));
        }
        Ok(serde_json::from_str(&text)?)
    }

    /// `POST /orders/dca/confirm-cancel/{id}`
    pub async fn confirm_cancel_dca_order(
        &self,
        access_token: &str,
        order_id: &str,
        body: &TriggerPriceOrderConfirmCancelRequest,
    ) -> Result<TriggerDcaOrderTxResponse> {
        let id = order_id.trim();
        if id.is_empty() {
            return Err(anyhow!("order_id required"));
        }
        let url = format!(
            "{}/orders/dca/confirm-cancel/{}",
            self.base_url.trim_end_matches('/'),
            id
        );
        let req = self
            .apply_api_key(self.client.post(url))?
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {access_token}"))
            .json(body);
        let res = req.send().await?;
        let status = res.status();
        let text = res.text().await?;
        if !status.is_success() {
            return Err(anyhow!("orders/dca/confirm-cancel {status}: {text}"));
        }
        Ok(serde_json::from_str(&text)?)
    }
}

/// Pure filter used by API/CLI and unit tests.
#[must_use]
/// Product role for a single Trigger price order (allowlista spot).
///
/// Jupiter has no separate SL endpoint — role follows mint orientation:
/// - **buy_dip**: `below` + `trigger_mint == output_mint` (buy asset on dip)
/// - **stop_loss**: `below` + `trigger_mint == input_mint` (sell asset if price falls)
/// - **take_profit**: `above` + `trigger_mint == input_mint` (sell asset if price rises)
/// - **buy_above**: `above` + `trigger_mint == output_mint` (breakout buy)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerPriceOrderRole {
    BuyDip,
    TakeProfit,
    StopLoss,
    BuyAbove,
    Other,
}

impl TriggerPriceOrderRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BuyDip => "buy_dip",
            Self::TakeProfit => "take_profit",
            Self::StopLoss => "stop_loss",
            Self::BuyAbove => "buy_above",
            Self::Other => "other",
        }
    }
}

/// Classify a price order from Jupiter mint + condition rules (static SL, not trailing).
pub fn classify_price_order_role(
    trigger_condition: &str,
    input_mint: &str,
    output_mint: &str,
    trigger_mint: &str,
) -> TriggerPriceOrderRole {
    let cond = trigger_condition.trim().to_ascii_lowercase();
    let input = input_mint.trim();
    let output = output_mint.trim();
    let trigger = trigger_mint.trim();
    if input.is_empty() || output.is_empty() || trigger.is_empty() {
        return TriggerPriceOrderRole::Other;
    }
    match cond.as_str() {
        "below" if trigger == output => TriggerPriceOrderRole::BuyDip,
        "below" if trigger == input => TriggerPriceOrderRole::StopLoss,
        "above" if trigger == input => TriggerPriceOrderRole::TakeProfit,
        "above" if trigger == output => TriggerPriceOrderRole::BuyAbove,
        _ => TriggerPriceOrderRole::Other,
    }
}

pub fn classify_history_item_role(o: &TriggerOrderHistoryItem) -> TriggerPriceOrderRole {
    classify_price_order_role(
        o.trigger_condition.as_deref().unwrap_or(""),
        o.input_mint.as_deref().unwrap_or(""),
        o.output_mint.as_deref().unwrap_or(""),
        o.trigger_mint.as_deref().unwrap_or(""),
    )
}

fn is_open_or_pending(state: &str) -> bool {
    state == "open" || state == "pending"
}

/// All open/pending price orders (Buy-the-Dip, TP, SL, …).
pub fn filter_open_price_orders(
    orders: &[TriggerOrderHistoryItem],
) -> Vec<TriggerOrderHistoryItem> {
    orders
        .iter()
        .filter(|o| is_open_or_pending(o.order_state.as_deref().unwrap_or("")))
        .cloned()
        .collect()
}

pub fn filter_open_buy_dips(orders: &[TriggerOrderHistoryItem]) -> Vec<TriggerOrderHistoryItem> {
    filter_open_price_orders(orders)
        .into_iter()
        .filter(|o| classify_history_item_role(o) == TriggerPriceOrderRole::BuyDip)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::Method::{GET, PATCH, POST};
    use httpmock::MockServer;

    #[test]
    fn filter_keeps_below_open_only() {
        let usdc = "USDC";
        let sol = "SOL";
        let orders = vec![
            TriggerOrderHistoryItem {
                id: "1".into(),
                order_type: Some("single".into()),
                order_state: Some("open".into()),
                input_mint: Some(usdc.into()),
                output_mint: Some(sol.into()),
                trigger_mint: Some(sol.into()),
                trigger_condition: Some("below".into()),
                trigger_price_usd: Some(90.5),
                initial_input_amount: None,
                remaining_input_amount: None,
                expires_at: None,
                created_at: None,
                updated_at: None,
            },
            TriggerOrderHistoryItem {
                id: "2".into(),
                order_type: Some("single".into()),
                order_state: Some("open".into()),
                input_mint: Some(sol.into()),
                output_mint: Some(usdc.into()),
                trigger_mint: Some(sol.into()),
                trigger_condition: Some("above".into()),
                trigger_price_usd: Some(110.0),
                initial_input_amount: None,
                remaining_input_amount: None,
                expires_at: None,
                created_at: None,
                updated_at: None,
            },
            TriggerOrderHistoryItem {
                id: "3".into(),
                order_type: Some("single".into()),
                order_state: Some("filled".into()),
                input_mint: Some(usdc.into()),
                output_mint: Some(sol.into()),
                trigger_mint: Some(sol.into()),
                trigger_condition: Some("below".into()),
                trigger_price_usd: Some(88.0),
                initial_input_amount: None,
                remaining_input_amount: None,
                expires_at: None,
                created_at: None,
                updated_at: None,
            },
            TriggerOrderHistoryItem {
                id: "4".into(),
                order_type: Some("single".into()),
                order_state: Some("pending".into()),
                input_mint: Some(sol.into()),
                output_mint: Some(usdc.into()),
                trigger_mint: Some(sol.into()),
                trigger_condition: Some("below".into()),
                trigger_price_usd: Some(70.0),
                initial_input_amount: None,
                remaining_input_amount: None,
                expires_at: None,
                created_at: None,
                updated_at: None,
            },
        ];
        assert_eq!(filter_open_price_orders(&orders).len(), 3);
        let dips = filter_open_buy_dips(&orders);
        assert_eq!(dips.len(), 1);
        assert_eq!(dips[0].id, "1");
        assert_eq!(
            classify_history_item_role(&orders[1]),
            TriggerPriceOrderRole::TakeProfit
        );
        assert_eq!(
            classify_history_item_role(&orders[3]),
            TriggerPriceOrderRole::StopLoss
        );
    }

    #[test]
    fn classify_roles_from_mint_orientation() {
        assert_eq!(
            classify_price_order_role("below", "USDC", "SOL", "SOL"),
            TriggerPriceOrderRole::BuyDip
        );
        assert_eq!(
            classify_price_order_role("below", "SOL", "USDC", "SOL"),
            TriggerPriceOrderRole::StopLoss
        );
        assert_eq!(
            classify_price_order_role("above", "SOL", "USDC", "SOL"),
            TriggerPriceOrderRole::TakeProfit
        );
        assert_eq!(
            classify_price_order_role("above", "USDC", "SOL", "SOL"),
            TriggerPriceOrderRole::BuyAbove
        );
    }

    #[tokio::test]
    async fn order_history_parses_with_mock() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET)
                .path("/orders/history")
                .header("x-api-key", "test-key")
                .header("Authorization", "Bearer jwt-1");
            then.status(200).json_body(serde_json::json!({
                "orders": [{
                    "id": "ord-a",
                    "orderType": "single",
                    "orderState": "open",
                    "inputMint": "USDC",
                    "outputMint": "SOL",
                    "triggerMint": "SOL",
                    "triggerCondition": "below",
                    "triggerPriceUsd": 90.5
                }],
                "pagination": { "total": 1, "limit": 20, "offset": 0 }
            }));
        });

        let client =
            JupiterTriggerClient::new(Some("test-key".into())).with_base_url(server.base_url());
        let hist = client
            .get_order_history(
                "jwt-1",
                &OrderHistoryQuery {
                    state: Some("active".into()),
                    limit: Some(20),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(hist.orders.len(), 1);
        assert_eq!(hist.orders[0].trigger_price_usd, Some(90.5));
        let dips = filter_open_buy_dips(&hist.orders);
        assert_eq!(dips.len(), 1);
    }

    #[tokio::test]
    async fn verify_parses_access_refresh() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST).path("/auth/verify");
            then.status(200).json_body(serde_json::json!({
                "accessToken": "acc",
                "refreshToken": "ref",
                "expiresAt": "2099-01-01T00:00:00Z",
                "tokenType": "Bearer"
            }));
        });
        let client = JupiterTriggerClient::new(Some("k".into())).with_base_url(server.base_url());
        let tokens = client
            .verify_challenge("Wallet111", "sig", "message")
            .await
            .unwrap();
        assert_eq!(tokens.access_token, "acc");
        assert_eq!(tokens.refresh_token.as_deref(), Some("ref"));
        assert_eq!(tokens.wallet_pubkey, "Wallet111");
    }

    #[tokio::test]
    async fn vault_and_craft_parse_with_mock() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET)
                .path("/vault")
                .header("Authorization", "Bearer jwt-v");
            then.status(200).json_body(serde_json::json!({
                "userPubkey": "User111",
                "vaultPubkey": "Vault222",
                "privyVaultId": "privy-1"
            }));
        });
        server.mock(|when, then| {
            when.method(POST)
                .path("/deposit/craft")
                .header("Authorization", "Bearer jwt-v");
            then.status(200).json_body(serde_json::json!({
                "transaction": "AQIDBA==",
                "requestId": "req-1",
                "receiverAddress": "Vault222",
                "mint": "So11111111111111111111111111111111111111112",
                "amount": "1000000",
                "tokenDecimals": 6
            }));
        });

        let client = JupiterTriggerClient::new(Some("k".into())).with_base_url(server.base_url());
        let vault = client.get_vault("jwt-v").await.unwrap();
        assert_eq!(vault.vault_pubkey, "Vault222");

        let craft = client
            .craft_deposit(
                "jwt-v",
                &TriggerDepositCraftRequest {
                    input_mint: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".into(),
                    output_mint: "So11111111111111111111111111111111111111112".into(),
                    user_address: "User111".into(),
                    amount: "1000000".into(),
                    order_type: "price".into(),
                    order_sub_type: Some("single".into()),
                },
            )
            .await
            .unwrap();
        assert_eq!(craft.request_id, "req-1");
        assert_eq!(craft.transaction, "AQIDBA==");
    }

    #[tokio::test]
    async fn create_price_order_parse_with_mock() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST)
                .path("/orders/price")
                .header("Authorization", "Bearer jwt-c");
            then.status(200).json_body(serde_json::json!({
                "id": "ord-42",
                "txSignature": "Sig111",
                "depositConfirmed": true
            }));
        });

        let client = JupiterTriggerClient::new(Some("k".into())).with_base_url(server.base_url());
        let created = client
            .create_price_order(
                "jwt-c",
                &TriggerPriceOrderCreateRequest {
                    order_type: "single".into(),
                    deposit_request_id: "req-1".into(),
                    deposit_signed_tx: "SIGNED==".into(),
                    user_pubkey: "User111".into(),
                    input_mint: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".into(),
                    output_mint: "So11111111111111111111111111111111111111112".into(),
                    input_amount: "10000000".into(),
                    trigger_mint: "So11111111111111111111111111111111111111112".into(),
                    trigger_condition: "below".into(),
                    trigger_price_usd: 90.5,
                    expires_at: 1_900_000_000_000,
                    slippage_bps: Some(100),
                },
            )
            .await
            .unwrap();
        assert_eq!(created.id, "ord-42");
        assert_eq!(created.tx_signature.as_deref(), Some("Sig111"));
        assert_eq!(created.deposit_confirmed, Some(true));
    }

    #[tokio::test]
    async fn cancel_and_confirm_parse_with_mock() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST)
                .path("/orders/price/cancel/ord-9")
                .header("Authorization", "Bearer jwt-x");
            then.status(200).json_body(serde_json::json!({
                "id": "ord-9",
                "transaction": "AQIDBA==",
                "requestId": "cancel-req-1"
            }));
        });
        server.mock(|when, then| {
            when.method(POST)
                .path("/orders/price/confirm-cancel/ord-9")
                .header("Authorization", "Bearer jwt-x");
            then.status(200).json_body(serde_json::json!({
                "id": "ord-9",
                "txSignature": "WithdrawSig"
            }));
        });

        let client = JupiterTriggerClient::new(Some("k".into())).with_base_url(server.base_url());
        let cancel = client.cancel_price_order("jwt-x", "ord-9").await.unwrap();
        assert_eq!(cancel.request_id, "cancel-req-1");
        assert_eq!(cancel.transaction, "AQIDBA==");

        let confirmed = client
            .confirm_cancel_price_order(
                "jwt-x",
                "ord-9",
                &TriggerPriceOrderConfirmCancelRequest {
                    signed_transaction: "SIGNED==".into(),
                    cancel_request_id: cancel.request_id,
                },
            )
            .await
            .unwrap();
        assert_eq!(confirmed.id, "ord-9");
        assert_eq!(confirmed.tx_signature.as_deref(), Some("WithdrawSig"));
    }

    #[tokio::test]
    async fn patch_price_order_parse_with_mock() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(PATCH)
                .path("/orders/price/ord-7")
                .header("Authorization", "Bearer jwt-p");
            then.status(200).json_body(serde_json::json!({
                "id": "ord-7"
            }));
        });

        let client = JupiterTriggerClient::new(Some("k".into())).with_base_url(server.base_url());
        let patched = client
            .patch_price_order(
                "jwt-p",
                "ord-7",
                &TriggerPriceOrderPatchRequest {
                    order_type: "single".into(),
                    trigger_price_usd: Some(88.25),
                    slippage_bps: Some(150),
                },
            )
            .await
            .unwrap();
        assert_eq!(patched.id, "ord-7");
    }

    #[tokio::test]
    async fn dca_create_history_cancel_parse_with_mock() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET)
                .path("/orders/history/dca")
                .header("Authorization", "Bearer jwt-d");
            then.status(200).json_body(serde_json::json!({
                "orders": [{
                    "id": "dca-1",
                    "orderState": "active",
                    "orderType": "time_based",
                    "inputMint": "USDC",
                    "outputMint": "SOL",
                    "inputAmount": "20000000",
                    "orderCount": 2,
                    "intervalSeconds": 3600,
                    "roundsRemaining": 2
                }],
                "pagination": { "total": 1, "limit": 20, "offset": 0 }
            }));
        });
        server.mock(|when, then| {
            when.method(POST)
                .path("/orders/dca")
                .header("Authorization", "Bearer jwt-d");
            then.status(200).json_body(serde_json::json!({
                "id": "dca-2",
                "txSignature": "DcaSig"
            }));
        });
        server.mock(|when, then| {
            when.method(POST)
                .path("/orders/dca/cancel/dca-2")
                .header("Authorization", "Bearer jwt-d");
            then.status(200).json_body(serde_json::json!({
                "id": "dca-2",
                "transaction": "AQIDBA==",
                "requestId": "dca-cancel-1",
                "roundsRemaining": 1,
                "refundAmount": "10000000"
            }));
        });
        server.mock(|when, then| {
            when.method(POST)
                .path("/orders/dca/confirm-cancel/dca-2")
                .header("Authorization", "Bearer jwt-d");
            then.status(200).json_body(serde_json::json!({
                "id": "dca-2",
                "txSignature": "WithdrawDca"
            }));
        });

        let client = JupiterTriggerClient::new(Some("k".into())).with_base_url(server.base_url());
        let hist = client
            .get_dca_order_history("jwt-d", &OrderHistoryQuery::default())
            .await
            .unwrap();
        assert_eq!(hist.orders.len(), 1);
        assert_eq!(hist.orders[0].id, "dca-1");

        let created = client
            .create_dca_order(
                "jwt-d",
                &TriggerDcaOrderCreateRequest {
                    deposit_request_id: "req".into(),
                    deposit_signed_tx: "SIGNED==".into(),
                    user_pubkey: "User".into(),
                    input_mint: "USDC".into(),
                    output_mint: "SOL".into(),
                    input_amount: "20000000".into(),
                    order_count: 2,
                    interval_seconds: 3600,
                    order_type: "time_based".into(),
                    min_price_usd: None,
                    max_price_usd: None,
                    trigger_mint: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(created.id, "dca-2");

        let cancel = client.cancel_dca_order("jwt-d", "dca-2").await.unwrap();
        assert_eq!(cancel.request_id, "dca-cancel-1");
        let confirmed = client
            .confirm_cancel_dca_order(
                "jwt-d",
                "dca-2",
                &TriggerPriceOrderConfirmCancelRequest {
                    signed_transaction: "SIGNED==".into(),
                    cancel_request_id: cancel.request_id,
                },
            )
            .await
            .unwrap();
        assert_eq!(confirmed.tx_signature, "WithdrawDca");
    }
}
