//! `serve` — run the stdio MCP proxy in front of an upstream HTTP MCP server.

use std::sync::Arc;

use anyhow::Context;
use rmcp::ServiceExt;
use rmcp::model::{ClientCapabilities, ClientInfo, Implementation};
use rmcp::transport::StreamableHttpClientTransport;

use crate::config::Config;
use crate::key::KeyResolver;
use crate::net::HttpUrl;
use crate::payment::AmountGuard;
use crate::proxy::X402Proxy;

pub async fn run(upstream: &HttpUrl, key_ref: String) -> anyhow::Result<()> {
    let config = Config::load().context("loading config")?;
    let guard = resolve_guard(&key_ref, &config)?;
    if matches!(guard, AmountGuard::Unset) {
        eprintln!(
            "[x402-proxy] warning: no spend ceiling (X402_MAX_AMOUNT or a wallet max) — all payments will be refused"
        );
    }

    let transport = StreamableHttpClientTransport::from_uri(upstream.as_str().to_string());
    let client_info = ClientInfo::new(
        ClientCapabilities::default(),
        Implementation::new("x402-proxy", env!("CARGO_PKG_VERSION")),
    );
    let upstream_svc = client_info
        .serve(transport)
        .await
        .with_context(|| format!("connecting to upstream {upstream}"))?;
    let tools = upstream_svc
        .list_all_tools()
        .await
        .context("listing upstream tools")?;
    eprintln!(
        "[x402-proxy] connected to {upstream} — {} tools",
        tools.len()
    );

    let proxy = X402Proxy::new(
        tools,
        upstream_svc.peer().clone(),
        Arc::new(KeyResolver::new(config)),
        key_ref,
        guard,
    );

    let service = proxy
        .serve(rmcp::transport::io::stdio())
        .await
        .context("starting stdio MCP server")?;
    eprintln!("[x402-proxy] stdio server ready");
    service.waiting().await?;
    Ok(())
}

/// Spend ceiling from `X402_MAX_AMOUNT`, falling back to the wallet's `max`
/// when `key_ref` is `wallet:<name>`.
fn resolve_guard(key_ref: &str, config: &Config) -> anyhow::Result<AmountGuard> {
    if let Ok(v) = std::env::var("X402_MAX_AMOUNT") {
        return AmountGuard::parse(&v).context("X402_MAX_AMOUNT");
    }
    if let Some(name) = key_ref.strip_prefix("wallet:")
        && let Ok(w) = config.wallet(name)
        && let Some(m) = &w.max
    {
        return AmountGuard::parse(m).context("wallet max");
    }
    Ok(AmountGuard::Unset)
}
