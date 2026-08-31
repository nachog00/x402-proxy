//! x402-proxy — stdio MCP server that proxies an upstream HTTP MCP server
//! and auto-signs x402 payments (USDC on Base, exact scheme).

pub mod key;
pub mod payment;
pub mod proxy;

/// Shared test fixtures (consts only). Hidden from docs; used by both unit and
/// integration tests so fixture values live in exactly one place.
#[doc(hidden)]
pub mod testkit;
