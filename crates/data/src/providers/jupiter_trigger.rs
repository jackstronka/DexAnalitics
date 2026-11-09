//! Jupiter Trigger Order API V2 — **read** client (auth + order history).
//!
//! Docs: <https://developers.jup.ag/docs/trigger>
//! Live create/cancel is **out of scope** (Dip Agent Faza E).

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
}

/// Pure filter used by API/CLI and unit tests.
#[must_use]
pub fn filter_open_buy_dips(orders: &[TriggerOrderHistoryItem]) -> Vec<TriggerOrderHistoryItem> {
    orders
        .iter()
        .filter(|o| {
            let state = o.order_state.as_deref().unwrap_or("");
            let cond = o.trigger_condition.as_deref().unwrap_or("");
            (state == "open" || state == "pending") && cond == "below"
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::Method::{GET, POST};
    use httpmock::MockServer;

    #[test]
    fn filter_keeps_below_open_only() {
        let orders = vec![
            TriggerOrderHistoryItem {
                id: "1".into(),
                order_type: Some("single".into()),
                order_state: Some("open".into()),
                input_mint: None,
                output_mint: None,
                trigger_mint: None,
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
                input_mint: None,
                output_mint: None,
                trigger_mint: None,
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
                input_mint: None,
                output_mint: None,
                trigger_mint: None,
                trigger_condition: Some("below".into()),
                trigger_price_usd: Some(88.0),
                initial_input_amount: None,
                remaining_input_amount: None,
                expires_at: None,
                created_at: None,
                updated_at: None,
            },
        ];
        let dips = filter_open_buy_dips(&orders);
        assert_eq!(dips.len(), 1);
        assert_eq!(dips[0].id, "1");
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
}
