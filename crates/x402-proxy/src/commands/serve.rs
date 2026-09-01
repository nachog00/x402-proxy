//! `serve` — run the stdio MCP proxy in front of an upstream HTTP MCP server.

use std::sync::Arc;

use anyhow::Context;
use rmcp::ServiceExt;
use rmcp::model::{ClientCapabilities, ClientInfo, Implementation};
use rmcp::transport::StreamableHttpClientTransport;

use crate::key::OpCli;
use crate::net::HttpUrl;
use crate::payment::AmountGuard;
use crate::proxy::X402Proxy;

pub async fn run(upstream: &HttpUrl, key_ref: String) -> anyhow::Result<()> {
    let guard = match std::env::var("X402_MAX_AMOUNT") {
        Ok(v) => AmountGuard::parse(&v).context("X402_MAX_AMOUNT")?,
        Err(_) => AmountGuard::Unset,
    };
    if matches!(guard, AmountGuard::Unset) {
        eprintln!("[x402-proxy] warning: X402_MAX_AMOUNT unset — all payments will be refused");
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
        Arc::new(OpCli::new()),
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
