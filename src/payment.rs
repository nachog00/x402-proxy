//! x402 payment types and payment-required detection.
//!
//! Wire data from the upstream is parsed tolerantly — upstream servers may
//! add fields at any time, so no `deny_unknown_fields` here (unlike our own
//! config structs).

pub mod exact;

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

const USDC_DECIMALS: u32 = 6;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid X402_MAX_AMOUNT '{0}' — expected decimal USDC like '0.50' (max 6 decimals)")]
    BadCeiling(String),
    #[error("X402_MAX_AMOUNT is not set — refusing to sign any payment")]
    CeilingUnset,
    #[error("payment of {amount_usdc} USDC exceeds X402_MAX_AMOUNT ({ceiling_usdc} USDC)", amount_usdc = fmt_usdc(*amount), ceiling_usdc = fmt_usdc(*ceiling))]
    OverCeiling { amount: u128, ceiling: u128 },
    #[error("unparseable payment amount '{0}'")]
    BadAmount(String),
    #[error("signing failed: {0}")]
    Signing(String),
}

/// Per-payment spending ceiling. `Unset` refuses to sign anything.
#[derive(Debug, PartialEq, Eq)]
pub enum AmountGuard {
    Unset,
    Max(u128),
}

impl AmountGuard {
    /// Parse a decimal USDC string ("0.50") into an atomic-unit ceiling.
    pub fn parse(s: &str) -> Result<Self, Error> {
        let bad = || Error::BadCeiling(s.to_string());
        let (whole, frac) = match s.split_once('.') {
            Some((w, f)) => (w, f),
            None => (s, ""),
        };
        if (whole.is_empty() && frac.is_empty())
            || frac.len() > USDC_DECIMALS as usize
            || (s.contains('.') && frac.is_empty())
        {
            return Err(bad());
        }
        let whole: u128 = if whole.is_empty() {
            0
        } else {
            whole.parse().map_err(|_| bad())?
        };
        let frac_atomic: u128 = if frac.is_empty() {
            0
        } else {
            let padded = format!("{frac:0<width$}", width = USDC_DECIMALS as usize);
            padded.parse().map_err(|_| bad())?
        };
        let atomic = whole
            .checked_mul(10u128.pow(USDC_DECIMALS))
            .and_then(|v| v.checked_add(frac_atomic))
            .ok_or_else(bad)?;
        Ok(Self::Max(atomic))
    }

    /// Check an atomic-unit amount string against the ceiling.
    pub fn check(&self, atomic: &str) -> Result<(), Error> {
        let amount: u128 = atomic
            .parse()
            .map_err(|_| Error::BadAmount(atomic.to_string()))?;
        match self {
            Self::Unset => Err(Error::CeilingUnset),
            Self::Max(ceiling) if amount > *ceiling => Err(Error::OverCeiling {
                amount,
                ceiling: *ceiling,
            }),
            Self::Max(_) => Ok(()),
        }
    }
}

/// Format atomic USDC units as a decimal string (at least 2 decimals).
pub fn fmt_usdc(atomic: u128) -> String {
    let scale = 10u128.pow(USDC_DECIMALS);
    let (whole, frac) = (atomic / scale, atomic % scale);
    let frac = format!("{frac:06}");
    let trimmed = frac.trim_end_matches('0');
    let frac = if trimmed.len() <= 2 { &frac[..2] } else { trimmed };
    format!("{whole}.{frac}")
}

/// Port: one way of satisfying an x402 payment demand.
pub trait PaymentScheme: Send + Sync {
    fn supports(&self, entry: &AcceptsEntry) -> bool;
    /// Produce the `_meta["x402/payment"]` JSON value.
    fn sign(&self, entry: &AcceptsEntry, x402_version: u32) -> Result<serde_json::Value, Error>;
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
        assert!(PaymentRequired::from_error_text(r#"{"accepts":[]}"#).is_none());
    }

    #[test]
    fn guard_parses_decimal_usdc() {
        assert_eq!(AmountGuard::parse("0.50").unwrap(), AmountGuard::Max(500_000));
        assert_eq!(AmountGuard::parse("1").unwrap(), AmountGuard::Max(1_000_000));
        assert_eq!(AmountGuard::parse("2.000001").unwrap(), AmountGuard::Max(2_000_001));
        assert_eq!(AmountGuard::parse(".5").unwrap(), AmountGuard::Max(500_000));
    }

    #[test]
    fn guard_rejects_bad_ceilings() {
        assert!(AmountGuard::parse("").is_err());
        assert!(AmountGuard::parse("abc").is_err());
        assert!(AmountGuard::parse("1.2345678").is_err()); // > 6 decimals
        assert!(AmountGuard::parse("-1").is_err());
        assert!(AmountGuard::parse("1.").is_err());
        // u128 overflow must fail closed, not wrap (multiplication overflow)
        assert!(AmountGuard::parse("340282366920938463463374607431769").is_err());
        // addition overflow at the exact multiplication threshold
        assert!(AmountGuard::parse("340282366920938463463374607431768.999999").is_err());
    }

    #[test]
    fn guard_checks_atomic_amounts() {
        let g = AmountGuard::Max(500_000); // 0.50 USDC
        assert!(g.check("499999").is_ok());
        assert!(g.check("500000").is_ok()); // at ceiling: allowed
        assert!(matches!(g.check("500001"), Err(Error::OverCeiling { .. })));
        assert!(g.check("not-a-number").is_err());
    }

    #[test]
    fn guard_unset_refuses_everything() {
        assert!(matches!(AmountGuard::Unset.check("1"), Err(Error::CeilingUnset)));
    }

    #[test]
    fn formats_atomic_as_decimal_usdc() {
        assert_eq!(fmt_usdc(500_000), "0.50");
        assert_eq!(fmt_usdc(1_000_000), "1.00");
        assert_eq!(fmt_usdc(2_000_001), "2.000001");
        assert_eq!(fmt_usdc(0), "0.00");
    }

    #[test]
    fn guard_check_survives_huge_wire_amounts() {
        let g = AmountGuard::Max(500_000);
        assert!(matches!(g.check(&"9".repeat(50)), Err(Error::BadAmount(_))));
    }
}
