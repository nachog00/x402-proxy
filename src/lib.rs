//! x402-proxy — stdio MCP server that proxies an upstream HTTP MCP server
//! and auto-signs x402 payments (USDC on Base, exact scheme).

pub mod key;
pub mod payment;
pub mod proxy;
