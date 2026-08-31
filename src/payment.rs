//! x402 payment types and payment-required detection.
//!
//! Wire data from the upstream is parsed tolerantly — upstream servers may
//! add fields at any time, so no `deny_unknown_fields` here (unlike our own
//! config structs).

pub mod exact;

use serde::Deserialize;

/// The x402 protocol version negotiated with the upstream. A bare integer on
/// the wire today; kept as a named alias so there is a single place to grow it
/// into a richer owned type if we ever need to reason about versions.
pub type X402Version = u32;

/// Parsed x402 payment-required error body.
#[derive(Debug, Deserialize)]
pub struct PaymentRequired {
    #[serde(rename = "x402Version", default = "default_version")]
    pub x402_version: X402Version,
    pub accepts: Vec<AcceptsEntry>,
}

fn default_version() -> X402Version {
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

/// Scheme hints supplied by the server. For `exact` these are the EIP-712
/// domain name/version; for `upto` they also carry the facilitator address the
/// Permit2 witness must be bound to.
#[derive(Debug, Default, Deserialize)]
pub struct Extra {
    pub name: Option<String>,
    pub version: Option<String>,
    /// Facilitator EOA for the `upto` (Permit2) scheme — the only address
    /// allowed to settle, bound into the signed witness.
    #[serde(rename = "facilitatorAddress")]
    pub facilitator_address: Option<String>,
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
    #[error("payment of {amount} USDC exceeds X402_MAX_AMOUNT ({ceiling} USDC)")]
    OverCeiling { amount: AtomicUsdc, ceiling: AtomicUsdc },
    #[error("unparseable payment amount '{0}'")]
    BadAmount(String),
    #[error("signing failed: {0}")]
    Signing(String),
}

/// A quantity of USDC in atomic units (6-decimal fixed point).
///
/// A newtype rather than a bare `u128` because it owns an invariant — the value
/// is a count of 1e-6 USDC — and a canonical decimal rendering. Parse raw input
/// into it once at the boundary (parse-don't-validate); nothing downstream ever
/// handles a bare amount string again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct AtomicUsdc(u128);

impl AtomicUsdc {
    /// Wrap a raw atomic-unit count — every `u128` is a valid count.
    pub const fn from_atomic(units: u128) -> Self {
        Self(units)
    }

    /// Parse a wire amount: integer atomic units as a decimal string
    /// ("1000000" = 1.00 USDC).
    pub fn parse_wire(s: &str) -> Result<Self, Error> {
        s.parse()
            .map(Self)
            .map_err(|_| Error::BadAmount(s.to_string()))
    }

    /// Parse a human ceiling: decimal USDC ("0.50"), at most 6 decimals.
    pub fn parse_decimal(s: &str) -> Result<Self, Error> {
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
        Ok(Self(atomic))
    }
}

impl std::fmt::Display for AtomicUsdc {
    /// Canonical decimal USDC, at least 2 fractional digits.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let scale = 10u128.pow(USDC_DECIMALS);
        let (whole, frac) = (self.0 / scale, self.0 % scale);
        let frac = format!("{frac:06}");
        let trimmed = frac.trim_end_matches('0');
        let frac = if trimmed.len() <= 2 { &frac[..2] } else { trimmed };
        write!(f, "{whole}.{frac}")
    }
}

/// Per-payment spending ceiling. `Unset` refuses to sign anything.
#[derive(Debug, PartialEq, Eq)]
pub enum AmountGuard {
    Unset,
    Max(AtomicUsdc),
}

impl AmountGuard {
    /// Parse a decimal USDC string ("0.50") into an atomic-unit ceiling.
    pub fn parse(s: &str) -> Result<Self, Error> {
        Ok(Self::Max(AtomicUsdc::parse_decimal(s)?))
    }

    /// Check an already-parsed demand against the ceiling. Parse-don't-validate:
    /// the caller turns the wire string into an `AtomicUsdc` once, then hands it
    /// here — this never touches a raw amount string.
    pub fn check(&self, amount: AtomicUsdc) -> Result<(), Error> {
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

/// Port: one way of satisfying an x402 payment demand.
pub trait PaymentScheme: Send + Sync {
    fn supports(&self, entry: &AcceptsEntry) -> bool;
    /// Produce the `_meta["x402/payment"]` JSON value.
    fn sign(&self, entry: &AcceptsEntry, x402_version: X402Version)
        -> Result<serde_json::Value, Error>;
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
        assert_eq!(e.extra.facilitator_address, None); // exact has none
    }

    // The `upto` (Permit2) entry Apify offers alongside `exact` (captured live).
    const APIFY_UPTO: &str = r#"{"x402Version":2,"accepts":[{"scheme":"upto","network":"eip155:8453","asset":"0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913","amount":"1000000","payTo":"0x4aAbE17C239eF71c3A26bA7C2b3e0AeBbfC1DF26","maxTimeoutSeconds":18000,"extra":{"name":"USD Coin","version":"2","facilitatorAddress":"0x14fDa13953Fc30428938E6BF950d036e77214e52"}}]}"#;

    #[test]
    fn parses_upto_entry_with_facilitator() {
        let pr = PaymentRequired::from_error_text(APIFY_UPTO).unwrap();
        let e = &pr.accepts[0];
        assert_eq!(e.scheme, "upto");
        assert_eq!(e.max_timeout_seconds, Some(18000));
        assert_eq!(
            e.extra.facilitator_address.as_deref(),
            Some("0x14fDa13953Fc30428938E6BF950d036e77214e52")
        );
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
        let max = |u| AmountGuard::Max(AtomicUsdc::from_atomic(u));
        assert_eq!(AmountGuard::parse("0.50").unwrap(), max(500_000));
        assert_eq!(AmountGuard::parse("1").unwrap(), max(1_000_000));
        assert_eq!(AmountGuard::parse("2.000001").unwrap(), max(2_000_001));
        assert_eq!(AmountGuard::parse(".5").unwrap(), max(500_000));
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
        let g = AmountGuard::Max(AtomicUsdc::from_atomic(500_000)); // 0.50 USDC
        let a = |s: &str| AtomicUsdc::parse_wire(s).unwrap();
        assert!(g.check(a("499999")).is_ok());
        assert!(g.check(a("500000")).is_ok()); // at ceiling: allowed
        assert!(matches!(g.check(a("500001")), Err(Error::OverCeiling { .. })));
    }

    #[test]
    fn guard_unset_refuses_everything() {
        assert!(matches!(
            AmountGuard::Unset.check(AtomicUsdc::from_atomic(1)),
            Err(Error::CeilingUnset)
        ));
    }

    #[test]
    fn formats_atomic_as_decimal_usdc() {
        let s = |u| AtomicUsdc::from_atomic(u).to_string();
        assert_eq!(s(500_000), "0.50");
        assert_eq!(s(1_000_000), "1.00");
        assert_eq!(s(2_000_001), "2.000001");
        assert_eq!(s(0), "0.00");
    }

    #[test]
    fn parse_wire_rejects_bad_and_out_of_range() {
        // parse-don't-validate: unparseable/oversized wire amounts fail at the
        // boundary, before the guard ever sees them.
        assert!(matches!(
            AtomicUsdc::parse_wire("not-a-number"),
            Err(Error::BadAmount(_))
        ));
        assert!(matches!(
            AtomicUsdc::parse_wire(&"9".repeat(50)),
            Err(Error::BadAmount(_))
        ));
    }
}
