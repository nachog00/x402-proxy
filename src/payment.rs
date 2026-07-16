//! x402 payment types and payment-required detection.
//!
//! Wire data from the upstream is parsed tolerantly — upstream servers may
//! add fields at any time, so no `deny_unknown_fields` here (unlike our own
//! config structs).

use serde::Deserialize;

/// Parsed x402 payment-required error body.
#[derive(Debug, Deserialize)]
pub struct PaymentRequired {
    #[serde(rename = "x402Version", default = "default_version")]
    pub x402_version: u32,
    pub accepts: Vec<AcceptsEntry>,
}

fn default_version() -> u32 {
    2
}

/// One entry of the `accepts` array: a payment option offered by the server.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptsEntry {
    pub scheme: String,
    pub network: String,
    /// Token contract address (USDC on Base).
    pub asset: String,
    /// Amount in the token's atomic units, as a decimal string.
    pub amount: String,
    pub pay_to: String,
    pub max_timeout_seconds: Option<u64>,
    #[serde(default)]
    pub extra: Extra,
}

/// EIP-712 domain hints supplied by the server.
#[derive(Debug, Default, Deserialize)]
pub struct Extra {
    pub name: Option<String>,
    pub version: Option<String>,
}

impl PaymentRequired {
    /// Detect a payment-required error in a tool result's text content.
    /// Returns None for anything that isn't x402 JSON with an `accepts` array.
    pub fn from_error_text(text: &str) -> Option<Self> {
        let pr: Self = serde_json::from_str(text).ok()?;
        (!pr.accepts.is_empty()).then_some(pr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured from a real mcp.apify.com?payment=x402 response (task brief).
    const APIFY_PAYLOAD: &str = r#"{"x402Version":2,"accepts":[{"scheme":"exact","network":"eip155:8453","asset":"0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913","amount":"1000000","payTo":"0x4aAbE17C239eF71c3A26bA7C2b3e0AeBbfC1DF26","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]}"#;

    #[test]
    fn parses_real_apify_payload() {
        let pr = PaymentRequired::from_error_text(APIFY_PAYLOAD).unwrap();
        assert_eq!(pr.x402_version, 2);
        assert_eq!(pr.accepts.len(), 1);
        let e = &pr.accepts[0];
        assert_eq!(e.scheme, "exact");
        assert_eq!(e.network, "eip155:8453");
        assert_eq!(e.asset, "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913");
        assert_eq!(e.amount, "1000000");
        assert_eq!(e.pay_to, "0x4aAbE17C239eF71c3A26bA7C2b3e0AeBbfC1DF26");
        assert_eq!(e.max_timeout_seconds, Some(60));
        assert_eq!(e.extra.name.as_deref(), Some("USD Coin"));
        assert_eq!(e.extra.version.as_deref(), Some("2"));
    }

    #[test]
    fn tolerates_unknown_fields_and_missing_optionals() {
        let text = r#"{"x402Version":2,"futureField":true,"accepts":[{"scheme":"exact","network":"eip155:8453","asset":"0xA","amount":"1","payTo":"0xB","surprise":1}]}"#;
        let pr = PaymentRequired::from_error_text(text).unwrap();
        assert_eq!(pr.accepts[0].max_timeout_seconds, None);
        assert!(pr.accepts[0].extra.name.is_none());
    }

    #[test]
    fn rejects_non_payment_errors() {
        assert!(PaymentRequired::from_error_text("upstream exploded").is_none());
        assert!(PaymentRequired::from_error_text(r#"{"error":"nope"}"#).is_none());
        assert!(PaymentRequired::from_error_text(r#"{"accepts":"not-an-array"}"#).is_none());
        assert!(PaymentRequired::from_error_text("").is_none());
    }
}
