//! The bridge: stdio MCP server in front, HTTP MCP client behind,
//! x402 sign-and-retry in the middle.

use std::sync::Arc;

use alloy_signer_local::PrivateKeySigner;
use rmcp::model::*;
use rmcp::service::{Peer, RequestContext, RoleClient, RoleServer};
use rmcp::{ErrorData as McpError, ServerHandler};
use tokio::sync::OnceCell;

use crate::key::SecretResolver;
use crate::payment::exact::{self, ExactEip3009};
use crate::payment::{AmountGuard, PaymentRequired, PaymentScheme};

pub struct X402Proxy {
    tools: Vec<Tool>,
    upstream: Peer<RoleClient>,
    resolver: Arc<dyn SecretResolver>,
    key_ref: String,
    guard: AmountGuard,
    scheme: OnceCell<ExactEip3009>,
}

impl X402Proxy {
    pub fn new(
        tools: Vec<Tool>,
        upstream: Peer<RoleClient>,
        resolver: Arc<dyn SecretResolver>,
        key_ref: String,
        guard: AmountGuard,
    ) -> Self {
        Self {
            tools,
            upstream,
            resolver,
            key_ref,
            guard,
            scheme: OnceCell::new(),
        }
    }

    /// Lazy signer: first payment triggers `op read`; cached for process life.
    async fn scheme(&self) -> Result<&ExactEip3009, String> {
        self.scheme
            .get_or_try_init(|| async {
                let resolver = self.resolver.clone();
                let key_ref = self.key_ref.clone();
                let key = tokio::task::spawn_blocking(move || resolver.resolve(&key_ref))
                    .await
                    .map_err(|e| format!("key resolution task failed: {e}"))?
                    .map_err(|e| e.to_string())?;
                let signer: PrivateKeySigner = key
                    .trim()
                    .parse()
                    .map_err(|_| "resolved key is not a valid EVM private key".to_string())?;
                eprintln!("[x402-proxy] signer ready: {}", signer.address());
                Ok(ExactEip3009::new(signer))
            })
            .await
    }

    /// The x402 dance. Returns a replacement result, or None to pass the
    /// original through.
    async fn try_pay_and_retry(
        &self,
        request: &CallToolRequestParams,
        result: &CallToolResult,
    ) -> Option<CallToolResult> {
        if result.is_error != Some(true) {
            return None;
        }
        // Upstream may return several content blocks — a JSON payment body
        // alongside a human-readable hint (Apify sends exactly this). Parse each
        // block on its own and take the first that is a valid payment-required
        // body; joining the blocks would corrupt the JSON and it would be missed.
        let pr = result
            .content
            .iter()
            .filter_map(|c| c.as_text())
            .find_map(|t| PaymentRequired::from_error_text(&t.text))?;

        // Static support check first — no key prompt for schemes we can't do.
        // Uses exact::supports (a free function) so selection and signing can
        // never diverge — the same predicate backs ExactEip3009::supports.
        let entry = match pr.accepts.iter().find(|e| exact::supports(e)) {
            Some(e) => e,
            None => {
                eprintln!(
                    "[x402-proxy] payment required for '{}' but no supported scheme — passing error through",
                    request.name
                );
                return None;
            }
        };

        // Parse the wire amount once (parse-don't-validate); the guard and the
        // log line both consume the typed value.
        let refuse = |e: crate::payment::Error| {
            Some(CallToolResult::error(vec![ContentBlock::text(format!(
                "x402-proxy refused to pay for '{}': {e}",
                request.name
            ))]))
        };
        let amount = match crate::payment::AtomicUsdc::parse_wire(&entry.amount) {
            Ok(a) => a,
            Err(e) => return refuse(e),
        };
        if let Err(e) = self.guard.check(amount) {
            return refuse(e);
        }

        eprintln!(
            "[x402-proxy] paying {amount} USDC to {} for '{}'",
            entry.pay_to, request.name
        );

        let scheme = match self.scheme().await {
            Ok(s) => s,
            Err(e) => {
                return Some(CallToolResult::error(vec![ContentBlock::text(format!(
                    "x402-proxy could not obtain signing key: {e}"
                ))]));
            }
        };
        let payment = match scheme.sign(entry, pr.x402_version) {
            Ok(p) => p,
            Err(e) => {
                return Some(CallToolResult::error(vec![ContentBlock::text(format!(
                    "x402-proxy signing failed: {e}"
                ))]));
            }
        };

        let mut retry = request.clone();
        let meta = retry.meta.get_or_insert_with(Meta::new);
        meta.insert("x402/payment".to_string(), payment);
        match self.upstream.call_tool(retry).await {
            Ok(r) => Some(r),
            Err(e) => Some(CallToolResult::error(vec![ContentBlock::text(format!(
                "x402-proxy retry after payment failed: {e}"
            ))])),
        }
    }
}

impl ServerHandler for X402Proxy {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info = Implementation::new("x402-proxy", env!("CARGO_PKG_VERSION"));
        info
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult {
            tools: self.tools.clone(),
            ..Default::default()
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let result = self
            .upstream
            .call_tool(request.clone())
            .await
            .map_err(|e| McpError::internal_error(format!("upstream call failed: {e}"), None))?;

        match self.try_pay_and_retry(&request, &result).await {
            Some(replacement) => Ok(replacement),
            None => Ok(result),
        }
    }
}
