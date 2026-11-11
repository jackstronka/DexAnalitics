//! Jupiter Swap API V2 — `/order` (quote + unsigned tx) + `/execute` (managed landing).
//!
//! Docs: <https://developers.jup.ag/docs/swap/order-and-execute>
//! Live execute gated by Dex API (`mode: live` + `confirm_execute`). Not for catching dips (use Trigger).

use anyhow::{Result, anyhow};
use reqwest::Client;
use serde::{Deserialize, Serialize};

const DEFAULT_BASE: &str = "https://api.jup.ag/swap/v2";

#[derive(Debug, Clone, Default)]
pub struct SwapOrderQuery {
    pub input_mint: String,
    pub output_mint: String,
    /// Amount in input mint smallest units.
    pub amount: String,
    /// Wallet that will sign; required for a buildable `transaction`.
    pub taker: Option<String>,
    pub slippage_bps: Option<u32>,
}

/// Response from `GET /order` (subset used by Dex).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SwapOrderResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_mint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_mint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_amount: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_amount: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_usd_value: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_usd_value: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_impact: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other_amount_threshold: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slippage_bps: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub router: Option<String>,
    /// Base64 VersionedTransaction; null without taker; empty string on build error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transaction: Option<String>,
    pub request_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SwapExecuteRequest {
    pub signed_transaction: String,
    pub request_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SwapExecuteResponse {
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_input_amount: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_output_amount: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_amount_result: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_amount_result: Option<String>,
}

#[derive(Debug, Clone)]
pub struct JupiterSwapClient {
    client: Client,
    base_url: String,
    api_key: Option<String>,
}

impl JupiterSwapClient {
    pub fn new(api_key: Option<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: DEFAULT_BASE.to_string(),
            api_key,
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    fn apply_api_key(&self, req: reqwest::RequestBuilder) -> Result<reqwest::RequestBuilder> {
        match &self.api_key {
            Some(k) if !k.trim().is_empty() => Ok(req.header("x-api-key", k.trim())),
            _ => Err(anyhow!("JUPITER_API_KEY required for Swap V2")),
        }
    }

    /// `GET /order` — quote (+ unsigned tx when `taker` set).
    pub async fn get_order(&self, q: &SwapOrderQuery) -> Result<SwapOrderResponse> {
        let mut url = format!("{}/order", self.base_url.trim_end_matches('/'));
        let mut params = vec![
            format!("inputMint={}", urlencoding_mint(&q.input_mint)),
            format!("outputMint={}", urlencoding_mint(&q.output_mint)),
            format!("amount={}", urlencoding_mint(&q.amount)),
        ];
        if let Some(t) = &q.taker
            && !t.trim().is_empty()
        {
            params.push(format!("taker={}", urlencoding_mint(t.trim())));
        }
        if let Some(bps) = q.slippage_bps {
            params.push(format!("slippageBps={bps}"));
        }
        url.push('?');
        url.push_str(&params.join("&"));

        let req = self
            .apply_api_key(self.client.get(&url))?
            .header("Accept", "application/json");
        let res = req.send().await?;
        let status = res.status();
        let text = res.text().await?;
        if !status.is_success() {
            return Err(anyhow!("swap/v2/order {status}: {text}"));
        }
        Ok(serde_json::from_str(&text)?)
    }

    /// `POST /execute` — land a signed `/order` transaction.
    pub async fn execute(&self, body: &SwapExecuteRequest) -> Result<SwapExecuteResponse> {
        let url = format!("{}/execute", self.base_url.trim_end_matches('/'));
        let req = self
            .apply_api_key(self.client.post(&url))?
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json(body);
        let res = req.send().await?;
        let status = res.status();
        let text = res.text().await?;
        if !status.is_success() {
            return Err(anyhow!("swap/v2/execute {status}: {text}"));
        }
        Ok(serde_json::from_str(&text)?)
    }
}

fn urlencoding_mint(s: &str) -> String {
    // Mint / amount / pubkey are URL-safe; keep simple encoding for `+` etc.
    s.trim()
        .replace('%', "%25")
        .replace('&', "%26")
        .replace('+', "%2B")
        .replace(' ', "%20")
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::Method::{GET, POST};
    use httpmock::MockServer;

    #[tokio::test]
    async fn order_and_execute_parse_with_mock() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET)
                .path("/order")
                .query_param("inputMint", "SOL")
                .query_param("outputMint", "USDC")
                .query_param("amount", "1000000")
                .query_param("taker", "Wallet111")
                .header("x-api-key", "test-key");
            then.status(200).json_body(serde_json::json!({
                "mode": "ultra",
                "inputMint": "SOL",
                "outputMint": "USDC",
                "inAmount": "1000000",
                "outAmount": "150000000",
                "inUsdValue": 150.0,
                "outUsdValue": 149.5,
                "priceImpact": -0.05,
                "otherAmountThreshold": "149000000",
                "slippageBps": 50,
                "router": "metis",
                "transaction": "AQIDBAUG",
                "requestId": "req-swap-1"
            }));
        });
        server.mock(|when, then| {
            when.method(POST)
                .path("/execute")
                .header("x-api-key", "test-key")
                .json_body(serde_json::json!({
                    "signedTransaction": "SIGNED",
                    "requestId": "req-swap-1"
                }));
            then.status(200).json_body(serde_json::json!({
                "status": "Success",
                "signature": "sig111",
                "code": 0,
                "totalInputAmount": "1000000",
                "totalOutputAmount": "149500000",
                "inputAmountResult": "1000000",
                "outputAmountResult": "149500000"
            }));
        });

        let client =
            JupiterSwapClient::new(Some("test-key".into())).with_base_url(server.base_url());
        let order = client
            .get_order(&SwapOrderQuery {
                input_mint: "SOL".into(),
                output_mint: "USDC".into(),
                amount: "1000000".into(),
                taker: Some("Wallet111".into()),
                slippage_bps: None,
            })
            .await
            .unwrap();
        assert_eq!(order.request_id, "req-swap-1");
        assert_eq!(order.out_amount.as_deref(), Some("150000000"));
        assert_eq!(order.transaction.as_deref(), Some("AQIDBAUG"));

        let exec = client
            .execute(&SwapExecuteRequest {
                signed_transaction: "SIGNED".into(),
                request_id: order.request_id,
            })
            .await
            .unwrap();
        assert_eq!(exec.status, "Success");
        assert_eq!(exec.signature.as_deref(), Some("sig111"));
    }
}
