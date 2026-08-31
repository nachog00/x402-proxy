//! Shared test fixtures — defined once so a hand-copied literal can't drift
//! between a signer, its assertions, and the cross-validation vector
//! generators in `proxies/x402/gen-*-vector.mjs`.
//!
//! `#[doc(hidden)]` and consts-only: nothing meaningful ships in the real
//! binary, but both unit tests and the integration tests in `tests/` can share
//! one source (a `#[cfg(test)]` module would be invisible to integration tests).

/// Throwaway signing key `0x…01` — publicly known, never funded.
pub const THROWAWAY_KEY: &str =
    "0x0000000000000000000000000000000000000000000000000000000000000001";
/// Address derived from [`THROWAWAY_KEY`] (EIP-55 checksummed).
pub const THROWAWAY_ADDR: &str = "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf";
/// USDC on Base — the only asset any scheme signs for.
pub const USDC: &str = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913";
/// Apify's payout recipient in the captured 402 responses.
pub const PAY_TO: &str = "0x4aAbE17C239eF71c3A26bA7C2b3e0AeBbfC1DF26";
/// Apify's `upto` facilitator EOA in the captured 402 responses.
pub const FACILITATOR: &str = "0x14fDa13953Fc30428938E6BF950d036e77214e52";
/// 1.00 USDC in atomic units.
pub const AMOUNT_1_USDC: &str = "1000000";

// Fixed timestamps for the cross-validation vectors. These MUST match the
// values baked into proxies/x402/gen-test-vector.mjs (exact) and
// gen-upto-vector.mjs (upto) — the Rust signer is asserted equal to those.
/// `validAfter` for both schemes' vectors.
pub const VALID_AFTER: u64 = 1_700_000_000;
/// `validBefore` — the `exact` scheme's expiry.
pub const VALID_BEFORE: u64 = 1_700_000_060;
/// `deadline` — the `upto` scheme's expiry.
pub const DEADLINE: u64 = 1_700_000_060;
