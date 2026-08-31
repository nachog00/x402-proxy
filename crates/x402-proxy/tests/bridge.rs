//! Integration: client ↔ X402Proxy ↔ mock upstream over in-process pipes.

use std::sync::{Arc, Mutex};

use rmcp::model::*;
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ErrorData as McpError, ServerHandler, ServiceExt};
use zeroize::Zeroizing;

use x402_proxy::key::{self, SecretResolver};
use x402_proxy::payment::{AmountGuard, AtomicUsdc};
use x402_proxy::proxy::X402Proxy;
use x402_proxy::testkit::THROWAWAY_ADDR;

const PAYMENT_JSON: &str = r#"{"x402Version":2,"accepts":[{"scheme":"exact","network":"eip155:8453","asset":"0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913","amount":"1000000","payTo":"0x4aAbE17C239eF71c3A26bA7C2b3e0AeBbfC1DF26","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]}"#;

/// Both schemes, as Apify offers them — `upto` first, then `exact`.
const BOTH_SCHEMES_JSON: &str = r#"{"x402Version":2,"accepts":[{"scheme":"upto","network":"eip155:8453","asset":"0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913","amount":"1000000","payTo":"0x4aAbE17C239eF71c3A26bA7C2b3e0AeBbfC1DF26","maxTimeoutSeconds":18000,"extra":{"name":"USD Coin","version":"2","facilitatorAddress":"0x14fDa13953Fc30428938E6BF950d036e77214e52"}},{"scheme":"exact","network":"eip155:8453","asset":"0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913","amount":"1000000","payTo":"0x4aAbE17C239eF71c3A26bA7C2b3e0AeBbfC1DF26","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]}"#;

/// Mock upstream: one tool, demands payment (with `payment_json`) until
/// `_meta["x402/payment"]` arrives.
#[derive(Clone)]
struct MockUpstream {
    seen_payments: Arc<Mutex<Vec<serde_json::Value>>>,
    payment_json: &'static str,
}

impl ServerHandler for MockUpstream {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let tool = Tool::new(
            "paid-echo",
            "echoes for money",
            serde_json::json!({"type": "object"})
                .as_object()
                .cloned()
                .unwrap(),
        );
        Ok(ListToolsResult {
            tools: vec![tool],
            ..Default::default()
        })
    }

    async fn call_tool(
        &self,
        _request: CallToolRequestParams,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        // rmcp routes request-level `_meta` into `ctx.meta` (not the params
        // struct's own `meta` field), so read the x402 payment from there.
        let payment = ctx.meta.get("x402/payment").cloned();
        match payment {
            Some(p) => {
                self.seen_payments.lock().unwrap().push(p);
                Ok(CallToolResult::success(vec![ContentBlock::text("paid ok")]))
            }
            // Real Apify returns the JSON body AND a human-readable hint as
            // two separate content blocks — mirror that so the proxy's
            // per-block parsing is exercised (joining them would break it).
            None => Ok(CallToolResult::error(vec![
                ContentBlock::text(self.payment_json),
                ContentBlock::text("Payment required to run this Actor or access this resource."),
            ])),
        }
    }
}

/// Resolver returning the throwaway key without touching 1Password.
struct FixedKey;
impl SecretResolver for FixedKey {
    fn resolve(&self, _r: &str) -> Result<Zeroizing<String>, key::Error> {
        Ok(Zeroizing::new(
            "0x0000000000000000000000000000000000000000000000000000000000000001".into(),
        ))
    }
}

/// General sandwich builder: any upstream handler, any resolver. Returns
/// the client and the upstream keep-alive guard (dropping it cancels the
/// upstream client and closes the transport, so tests must hold it for
/// their whole body).
async fn spawn_sandwich_full<H, R>(
    upstream_handler: H,
    resolver: R,
    guard: AmountGuard,
) -> (
    impl std::ops::Deref<Target = rmcp::service::Peer<rmcp::service::RoleClient>>,
    impl Send,
)
where
    H: ServerHandler + 'static,
    R: SecretResolver + 'static,
{
    // upstream server <-> proxy's client
    let (upstream_side, proxy_client_side) = tokio::io::duplex(1 << 16);
    tokio::spawn(async move {
        let svc = upstream_handler
            .serve(tokio::io::split(upstream_side))
            .await
            .unwrap();
        svc.waiting().await.ok();
    });
    let upstream =
        ().serve(tokio::io::split(proxy_client_side))
            .await
            .expect("connect to mock upstream");
    let tools = upstream.list_all_tools().await.unwrap();

    let proxy = X402Proxy::new(
        tools,
        upstream.peer().clone(),
        Arc::new(resolver),
        "op://unused/ref".into(),
        guard,
    );

    // test client <-> proxy's server
    let (proxy_server_side, client_side) = tokio::io::duplex(1 << 16);
    tokio::spawn(async move {
        let svc = proxy
            .serve(tokio::io::split(proxy_server_side))
            .await
            .unwrap();
        svc.waiting().await.ok();
    });
    let client = ().serve(tokio::io::split(client_side)).await.unwrap();
    (client, upstream)
}

async fn spawn_sandwich(
    guard: AmountGuard,
) -> (
    impl std::ops::Deref<Target = rmcp::service::Peer<rmcp::service::RoleClient>>,
    Arc<Mutex<Vec<serde_json::Value>>>,
    impl Send,
) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mock = MockUpstream {
        seen_payments: seen.clone(),
        payment_json: PAYMENT_JSON,
    };
    let (client, upstream) = spawn_sandwich_full(mock, FixedKey, guard).await;
    (client, seen, upstream)
}

#[tokio::test]
async fn lists_upstream_tools() {
    let (client, _seen, _upstream) =
        spawn_sandwich(AmountGuard::Max(AtomicUsdc::from_atomic(2_000_000))).await;
    let tools = client.list_all_tools().await.unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "paid-echo");
}

#[tokio::test]
async fn signs_and_retries_on_payment_required() {
    let (client, seen, _upstream) =
        spawn_sandwich(AmountGuard::Max(AtomicUsdc::from_atomic(2_000_000))).await;
    let result = client
        .call_tool(CallToolRequestParams::new("paid-echo"))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true));

    let payments = seen.lock().unwrap();
    assert_eq!(payments.len(), 1);
    let p = &payments[0];
    assert_eq!(p["scheme"], "exact");
    assert_eq!(p["network"], "eip155:8453");
    assert_eq!(p["payload"]["authorization"]["from"], THROWAWAY_ADDR);
    let sig = p["payload"]["signature"].as_str().unwrap();
    assert!(sig.starts_with("0x") && sig.len() == 132);
}

#[tokio::test]
async fn prefers_upto_when_both_schemes_offered() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mock = MockUpstream {
        seen_payments: seen.clone(),
        payment_json: BOTH_SCHEMES_JSON,
    };
    let (client, _upstream) = spawn_sandwich_full(
        mock,
        FixedKey,
        AmountGuard::Max(AtomicUsdc::from_atomic(2_000_000)),
    )
    .await;

    let result = client
        .call_tool(CallToolRequestParams::new("paid-echo"))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true));

    let payments = seen.lock().unwrap();
    assert_eq!(payments.len(), 1);
    let p = &payments[0];
    assert_eq!(p["scheme"], "upto", "must prefer upto over exact");
    assert!(
        p["payload"]["permit2Authorization"].is_object(),
        "upto payload carries permit2Authorization"
    );
    let sig = p["payload"]["signature"].as_str().unwrap();
    assert!(sig.starts_with("0x") && sig.len() == 132);
}

#[tokio::test]
async fn refuses_over_ceiling_without_signing() {
    // ceiling 0.50 USDC < demanded 1.00 USDC
    let (client, seen, _upstream) =
        spawn_sandwich(AmountGuard::Max(AtomicUsdc::from_atomic(500_000))).await;
    let result = client
        .call_tool(CallToolRequestParams::new("paid-echo"))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    let text = result.content[0].as_text().unwrap().text.clone();
    assert!(text.contains("X402_MAX_AMOUNT"), "got: {text}");
    assert!(
        seen.lock().unwrap().is_empty(),
        "must not sign over ceiling"
    );
}

#[tokio::test]
async fn refuses_when_ceiling_unset() {
    let (client, seen, _upstream) = spawn_sandwich(AmountGuard::Unset).await;
    let result = client
        .call_tool(CallToolRequestParams::new("paid-echo"))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(seen.lock().unwrap().is_empty());
}

/// Upstream that always errors with a non-x402 message — used to verify the
/// proxy passes ordinary tool errors through untouched, without attempting
/// to parse or sign anything.
#[derive(Clone)]
struct BrokenUpstream;

impl ServerHandler for BrokenUpstream {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let tool = Tool::new(
            "paid-echo",
            "echoes for money",
            serde_json::json!({"type": "object"})
                .as_object()
                .cloned()
                .unwrap(),
        );
        Ok(ListToolsResult {
            tools: vec![tool],
            ..Default::default()
        })
    }

    async fn call_tool(
        &self,
        _request: CallToolRequestParams,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::error(vec![ContentBlock::text(
            "upstream exploded: disk full",
        )]))
    }
}

#[tokio::test]
async fn non_payment_error_passes_through_untouched() {
    let (client, _upstream) = spawn_sandwich_full(
        BrokenUpstream,
        FixedKey,
        AmountGuard::Max(AtomicUsdc::from_atomic(2_000_000)),
    )
    .await;
    let result = client
        .call_tool(CallToolRequestParams::new("paid-echo"))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    let text = result.content[0].as_text().unwrap().text.clone();
    assert_eq!(text, "upstream exploded: disk full");
}

/// Resolver that always fails — simulates `op read` being unavailable (or
/// returning nothing), without touching a real `op` binary.
struct FailingKey;
impl SecretResolver for FailingKey {
    fn resolve(&self, _r: &str) -> Result<Zeroizing<String>, key::Error> {
        Err(key::Error::Empty)
    }
}

#[tokio::test]
async fn key_resolution_failure_surfaces_as_tool_error() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mock = MockUpstream {
        seen_payments: seen.clone(),
        payment_json: PAYMENT_JSON,
    };
    // Permissive ceiling: the guard check must pass so we actually reach
    // key resolution rather than being rejected earlier.
    let (client, _upstream) = spawn_sandwich_full(
        mock,
        FailingKey,
        AmountGuard::Max(AtomicUsdc::from_atomic(2_000_000)),
    )
    .await;
    let result = client
        .call_tool(CallToolRequestParams::new("paid-echo"))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    let text = result.content[0].as_text().unwrap().text.clone();
    assert!(text.contains("could not obtain signing key"), "got: {text}");
    // Upstream still demanded payment (its own error), but the proxy never
    // got far enough to sign and retry.
    assert!(seen.lock().unwrap().is_empty());
}
