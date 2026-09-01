//! `install` — register this proxy in front of an upstream MCP server.
//!
//! Builds the stdio invocation (`x402-proxy serve --upstream <url>`) plus the
//! `X402_KEY_REF` / `X402_MAX_AMOUNT` env the proxy needs, then either prints
//! portable `mcpServers` JSON (the default, paste into any client) or shells out
//! to `claude mcp add`. The key is never resolved here — only the *reference*
//! (a `wallet:NAME`, `op://…`, `env:VAR`, …) is written into the client config,
//! so no secret ever touches argv or stdout.

use std::collections::BTreeMap;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::json;

use crate::cli::{InstallClient, InstallScope};
use crate::config::{Config, KeySource};
use crate::net::HttpUrl;

/// A resolved MCP stdio server entry, ready to print or hand to `claude mcp add`.
#[derive(Debug, PartialEq, Eq)]
struct McpServer {
    name: String,
    command: String,
    args: Vec<String>,
    env: BTreeMap<String, String>,
}

/// Build the server entry from CLI inputs and the config. Pure — no I/O beyond
/// the already-loaded `config` — so it's unit-tested directly.
///
/// `X402_KEY_REF` comes from `--wallet` (→ `wallet:NAME`), an explicit
/// `--key-ref`, or the config's `default_wallet`. `X402_MAX_AMOUNT` comes from
/// `--max`, falling back to the referenced wallet's `max`.
fn plan(
    upstream: &HttpUrl,
    name: &str,
    wallet: Option<&str>,
    key_ref: Option<&str>,
    max: Option<&str>,
    config: &Config,
) -> Result<McpServer> {
    let key_ref = resolve_key_ref(wallet, key_ref, config)?;

    // The ceiling: explicit --max wins; otherwise inherit the wallet's max when
    // the ref points at a config wallet.
    let ceiling = match max {
        Some(m) => Some(m.to_string()),
        None => wallet_name(&key_ref)
            .and_then(|n| config.wallet(n).ok())
            .and_then(|w| w.max.clone()),
    };

    let url = upstream.with_payment_param();
    let mut env = BTreeMap::new();
    env.insert("X402_KEY_REF".to_string(), key_ref);
    if let Some(c) = ceiling {
        env.insert("X402_MAX_AMOUNT".to_string(), c);
    }

    Ok(McpServer {
        name: name.to_string(),
        command: "x402-proxy".to_string(),
        args: vec![
            "serve".to_string(),
            "--upstream".to_string(),
            url.as_str().to_string(),
        ],
        env,
    })
}

/// Decide the `X402_KEY_REF` string and validate it. `--wallet`/`--key-ref` are
/// mutually exclusive at the CLI; falling through to `default_wallet` keeps the
/// common single-wallet setup zero-flag.
fn resolve_key_ref(wallet: Option<&str>, key_ref: Option<&str>, config: &Config) -> Result<String> {
    if let Some(name) = wallet {
        config
            .wallet(name)
            .with_context(|| format!("--wallet {name}"))?;
        return Ok(format!("wallet:{name}"));
    }
    if let Some(r) = key_ref {
        // A wallet: ref must name a real wallet; any other source must parse.
        if let Some(name) = wallet_name(r) {
            config
                .wallet(name)
                .with_context(|| format!("--key-ref {r}"))?;
        } else {
            r.parse::<KeySource>()
                .with_context(|| format!("--key-ref {r}"))?;
        }
        return Ok(r.to_string());
    }
    match &config.default_wallet {
        Some(name) => {
            config
                .wallet(name)
                .with_context(|| format!("default_wallet '{name}'"))?;
            Ok(format!("wallet:{name}"))
        }
        None => bail!(
            "no key source — pass --wallet <name>, --key-ref <source>, or set default_wallet in the config"
        ),
    }
}

/// The `NAME` in a `wallet:NAME` ref, if that's the shape.
fn wallet_name(key_ref: &str) -> Option<&str> {
    key_ref.strip_prefix("wallet:")
}

/// Portable `mcpServers` JSON — the shape Claude Desktop and most clients accept.
fn mcp_json(server: &McpServer) -> serde_json::Value {
    json!({
        "mcpServers": {
            &server.name: {
                "command": server.command,
                "args": server.args,
                "env": server.env,
            }
        }
    })
}

/// Run `claude mcp add <name> --scope <s> --transport stdio -e K=V … -- <command> <args…>`.
fn run_claude_mcp_add(server: &McpServer, scope: InstallScope) -> Result<()> {
    let mut cmd = Command::new("claude");
    cmd.arg("mcp")
        .arg("add")
        .arg(&server.name)
        .arg("--scope")
        .arg(scope.as_claude_flag())
        .arg("--transport")
        .arg("stdio");
    for (k, v) in &server.env {
        cmd.arg("-e").arg(format!("{k}={v}"));
    }
    cmd.arg("--").arg(&server.command).args(&server.args);

    let status = cmd
        .status()
        .context("running `claude mcp add` — is the Claude CLI on PATH?")?;
    if !status.success() {
        bail!("`claude mcp add` exited with {status}");
    }
    eprintln!(
        "[x402-proxy] registered '{}' with Claude ({} scope)",
        server.name,
        scope.as_claude_flag()
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    upstream: &HttpUrl,
    name: &str,
    wallet: Option<&str>,
    key_ref: Option<&str>,
    max: Option<&str>,
    client: InstallClient,
    scope: InstallScope,
) -> Result<()> {
    let config = Config::load().context("loading config")?;
    let server = plan(upstream, name, wallet, key_ref, max, &config)?;

    if !server.env.contains_key("X402_MAX_AMOUNT") {
        eprintln!(
            "[x402-proxy] note: no spend ceiling set (no --max and no wallet max) — the proxy will refuse all payments until X402_MAX_AMOUNT is set"
        );
    }

    match client {
        InstallClient::Print => {
            println!("{}", serde_json::to_string_pretty(&mcp_json(&server))?);
            Ok(())
        }
        InstallClient::Claude => run_claude_mcp_add(&server, scope),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        toml::from_str(
            r#"
            default_wallet = "main"
            [wallets.main]
            key = "op://V/i/f"
            max = "0.50"
            [wallets.bare]
            key = "env:X402_KEY"
            "#,
        )
        .unwrap()
    }

    fn upstream() -> HttpUrl {
        "https://mcp.apify.com".parse().unwrap()
    }

    #[test]
    fn wallet_flag_sets_ref_and_inherits_max() {
        let s = plan(&upstream(), "x402", Some("main"), None, None, &cfg()).unwrap();
        assert_eq!(s.env["X402_KEY_REF"], "wallet:main");
        assert_eq!(s.env["X402_MAX_AMOUNT"], "0.50");
        // bare host got the payment param appended
        assert_eq!(
            s.args,
            vec!["serve", "--upstream", "https://mcp.apify.com/?payment=x402"]
        );
    }

    #[test]
    fn explicit_max_overrides_wallet_max() {
        let s = plan(&upstream(), "x402", Some("main"), None, Some("2.5"), &cfg()).unwrap();
        assert_eq!(s.env["X402_MAX_AMOUNT"], "2.5");
    }

    #[test]
    fn defaults_to_default_wallet_when_no_flags() {
        let s = plan(&upstream(), "x402", None, None, None, &cfg()).unwrap();
        assert_eq!(s.env["X402_KEY_REF"], "wallet:main");
    }

    #[test]
    fn key_ref_source_passthrough_no_ceiling() {
        let s = plan(
            &upstream(),
            "x402",
            None,
            Some("env:MY_KEY"),
            None,
            &Config::default(),
        )
        .unwrap();
        assert_eq!(s.env["X402_KEY_REF"], "env:MY_KEY");
        // no --max and not a config wallet → no ceiling emitted
        assert!(!s.env.contains_key("X402_MAX_AMOUNT"));
    }

    #[test]
    fn key_ref_wallet_inherits_its_max() {
        let s = plan(&upstream(), "x402", None, Some("wallet:main"), None, &cfg()).unwrap();
        assert_eq!(s.env["X402_KEY_REF"], "wallet:main");
        assert_eq!(s.env["X402_MAX_AMOUNT"], "0.50");
    }

    #[test]
    fn rejects_unknown_wallet_and_bad_source() {
        assert!(plan(&upstream(), "x402", Some("nope"), None, None, &cfg()).is_err());
        assert!(plan(&upstream(), "x402", None, Some("wallet:nope"), None, &cfg()).is_err());
        assert!(
            plan(
                &upstream(),
                "x402",
                None,
                Some("garbage"),
                None,
                &Config::default()
            )
            .is_err()
        );
    }

    #[test]
    fn no_key_source_at_all_is_an_error() {
        assert!(plan(&upstream(), "x402", None, None, None, &Config::default()).is_err());
    }

    #[test]
    fn mcp_json_shape() {
        let s = plan(&upstream(), "apify", Some("main"), None, None, &cfg()).unwrap();
        let v = mcp_json(&s);
        let entry = &v["mcpServers"]["apify"];
        assert_eq!(entry["command"], "x402-proxy");
        assert_eq!(entry["args"][0], "serve");
        assert_eq!(entry["env"]["X402_KEY_REF"], "wallet:main");
    }
}
