//! Key resolution — a scheme-tagged `X402_KEY_REF` to in-memory key material.
//!
//! Sources: `op://…` (1Password), `env:VAR`, `file:/path`, `wallet:NAME`
//! (looked up in the config), or a raw `0x…` key (throwaway/low-stakes only).
//! The resolved value lives in `Zeroizing<String>` and must never be logged,
//! written to disk, or placed in argv. The ref itself is also kept out of logs
//! (op:// refs leak vault structure).

use std::path::Path;

use zeroize::Zeroizing;

use crate::config::{Config, KeySource};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to run `op` — is the 1Password CLI installed and on PATH?")]
    Spawn(#[source] std::io::Error),
    #[error("`op read` failed ({status}): {stderr}")]
    OpFailed { status: String, stderr: String },
    #[error("`op read` returned empty output")]
    Empty,
    #[error("X402_KEY_REF is not set — no key source to resolve")]
    MissingRef,
    #[error("env var '{0}' (from env:) is not set")]
    EnvMissing(String),
    #[error("reading key file '{path}'")]
    FileRead {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    BadSource(#[from] crate::config::BadKeySource),
    #[error(transparent)]
    Config(#[from] crate::config::Error),
}

/// Port: resolve a secret reference to in-memory key material.
///
/// Blocking call — wrap in `spawn_blocking` from async contexts.
pub trait SecretResolver: Send + Sync {
    fn resolve(&self, secret_ref: &str) -> Result<Zeroizing<String>, Error>;
}

/// 1Password CLI backend: `op read <ref>`.
pub struct OpCli {
    program: String,
}

impl OpCli {
    pub fn new() -> Self {
        Self::with_program("op")
    }

    fn with_program(program: &str) -> Self {
        Self {
            program: program.to_string(),
        }
    }
}

impl Default for OpCli {
    fn default() -> Self {
        Self::new()
    }
}

/// op echoes the full secret ref in its error output; scrub it so
/// `OpFailed`'s Display never leaks vault structure.
fn redact(stderr: &str, secret_ref: &str) -> String {
    stderr.replace(secret_ref, "[redacted-ref]")
}

impl SecretResolver for OpCli {
    fn resolve(&self, secret_ref: &str) -> Result<Zeroizing<String>, Error> {
        let mut output = std::process::Command::new(&self.program)
            .arg("read")
            .arg(secret_ref)
            .output()
            .map_err(Error::Spawn)?;
        // Wipe-on-drop from this point, on every path.
        let stdout = Zeroizing::new(std::mem::take(&mut output.stdout));
        if !output.status.success() {
            return Err(Error::OpFailed {
                status: output.status.to_string(),
                stderr: redact(String::from_utf8_lossy(&output.stderr).trim(), secret_ref),
            });
        }
        let mut secret = Zeroizing::new(String::from_utf8_lossy(&stdout).into_owned());
        let end = secret.trim_end().len();
        secret.truncate(end);
        if secret.is_empty() {
            return Err(Error::Empty);
        }
        Ok(secret)
    }
}

/// Resolves a scheme-tagged `X402_KEY_REF` via the right backend, using the
/// config only for `wallet:` lookups. This is what the proxy and the
/// `approve-permit2` command use.
pub struct KeyResolver {
    config: Config,
}

impl KeyResolver {
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    /// Resolve a concrete [`KeySource`] to key material. `wallet:` never reaches
    /// here — the enum has no such variant, so nesting is impossible by type.
    fn resolve_source(&self, source: &KeySource) -> Result<Zeroizing<String>, Error> {
        match source {
            // op read wants the full op:// ref.
            KeySource::Op(r) => OpCli::new().resolve(r),
            KeySource::Env(name) => {
                let v = std::env::var(name).map_err(|_| Error::EnvMissing(name.clone()))?;
                Ok(Zeroizing::new(v.trim().to_string()))
            }
            KeySource::File(path) => read_key_file(path),
            KeySource::Raw(k) => {
                eprintln!(
                    "[x402-proxy] warning: using a raw inline key — fine for a throwaway/low-stakes wallet, but prefer op://, env:, or file: for real funds"
                );
                Ok(Zeroizing::new(k.clone()))
            }
        }
    }
}

/// Read a key from a file, trimming trailing whitespace.
fn read_key_file(path: &Path) -> Result<Zeroizing<String>, Error> {
    let v = std::fs::read_to_string(path).map_err(|source| Error::FileRead {
        path: path.display().to_string(),
        source,
    })?;
    Ok(Zeroizing::new(v.trim().to_string()))
}

impl SecretResolver for KeyResolver {
    fn resolve(&self, key_ref: &str) -> Result<Zeroizing<String>, Error> {
        let r = key_ref.trim();
        if r.is_empty() {
            return Err(Error::MissingRef);
        }
        // `wallet:NAME` indirects through the config to a concrete KeySource.
        // The looked-up `key` is already a KeySource, so it can never be another
        // `wallet:` — nesting is ruled out at the type level.
        if let Some(name) = r.strip_prefix("wallet:") {
            let source = self.config.wallet(name)?.key.clone();
            return self.resolve_source(&source);
        }
        self.resolve_source(&r.parse::<KeySource>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn op_cli_reports_missing_binary() {
        let r = OpCli::with_program("definitely-not-a-real-binary-xyz");
        let err = r.resolve("op://Vault/item/field").unwrap_err();
        assert!(matches!(err, Error::Spawn(_)));
    }

    #[test]
    fn op_cli_reports_failure_with_stderr() {
        // `false` exits 1 with no output; simulates a locked/failed op.
        let r = OpCli::with_program("false");
        let err = r.resolve("op://Vault/item/field").unwrap_err();
        assert!(matches!(err, Error::OpFailed { .. }));
    }

    #[test]
    fn op_cli_trims_trailing_newline() {
        // `echo` stands in for op: prints the "secret" plus a newline.
        let r = OpCli::with_program("echo");
        let key = r.resolve("0xabc").unwrap();
        assert_eq!(key.as_str(), "read 0xabc");
    }

    #[test]
    fn op_cli_rejects_empty_output() {
        // `true` exits 0 with no stdout: a "successful" blank read must fail.
        let r = OpCli::with_program("true");
        assert!(matches!(
            r.resolve("op://Vault/item/field").unwrap_err(),
            Error::Empty
        ));
    }

    #[test]
    fn redact_scrubs_ref_from_stderr() {
        let out = redact(
            "[ERROR] could not read secret 'op://Private/wallet/key': not found",
            "op://Private/wallet/key",
        );
        assert!(!out.contains("op://Private/wallet/key"));
        assert!(out.contains("[redacted-ref]"));
    }

    #[test]
    fn resolver_raw_key_passthrough() {
        let r = KeyResolver::new(Config::default());
        assert_eq!(r.resolve("0xabc123").unwrap().as_str(), "0xabc123");
    }

    #[test]
    fn resolver_env_source() {
        // SAFETY: unique var name; no other test reads it.
        unsafe { std::env::set_var("X402_TEST_KEY_ENV_SRC", "0xdeadbeef") };
        let r = KeyResolver::new(Config::default());
        assert_eq!(
            r.resolve("env:X402_TEST_KEY_ENV_SRC").unwrap().as_str(),
            "0xdeadbeef"
        );
        assert!(matches!(
            r.resolve("env:X402_TEST_MISSING_XYZ_123"),
            Err(Error::EnvMissing(_))
        ));
    }

    #[test]
    fn resolver_file_source() {
        let p = std::env::temp_dir().join(format!("x402-key-{}.txt", std::process::id()));
        std::fs::write(&p, "0xfilekey\n").unwrap();
        let r = KeyResolver::new(Config::default());
        assert_eq!(
            r.resolve(&format!("file:{}", p.display()))
                .unwrap()
                .as_str(),
            "0xfilekey"
        );
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn resolver_wallet_indirection() {
        let cfg: Config = toml::from_str("[wallets.main]\nkey = \"0xwalletkey\"").unwrap();
        let r = KeyResolver::new(cfg);
        assert_eq!(r.resolve("wallet:main").unwrap().as_str(), "0xwalletkey");
        assert!(matches!(r.resolve("wallet:nope"), Err(Error::Config(_))));
    }

    // A wallet whose `key` is itself `wallet:…` is unrepresentable: `KeySource`
    // has no wallet variant, so config parsing rejects it before it ever reaches
    // the resolver (covered by config::tests::rejects_unknown_fields_and_bad_source).

    #[test]
    fn resolver_unknown_and_missing() {
        let r = KeyResolver::new(Config::default());
        assert!(matches!(r.resolve("garbage"), Err(Error::BadSource(_))));
        assert!(matches!(r.resolve(""), Err(Error::MissingRef)));
        assert!(matches!(r.resolve("   "), Err(Error::MissingRef)));
    }
}
