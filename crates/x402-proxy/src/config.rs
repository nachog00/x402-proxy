//! Optional config file (`~/.config/x402-proxy/config.toml`): define wallets
//! once — a key source plus a default ceiling — and reference them by name via
//! `X402_KEY_REF=wallet:<name>` or `x402-proxy install --wallet <name>`.
//!
//! ```toml
//! default_wallet = "main"
//!
//! [wallets.main]
//! key = "op://Private/x402-wallet/private-key"   # any key source
//! max = "0.50"                                    # default USDC ceiling
//!
//! [wallets.dev]
//! key = "env:X402_DEV_KEY"
//! ```

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::str::FromStr;

use serde::Deserialize;

/// A primitive key source. Deliberately has no `wallet:` variant, so a wallet's
/// `key` can never point at another wallet (enforced at the type level).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeySource {
    /// 1Password reference, e.g. `op://Vault/item/field`.
    Op(String),
    /// Name of an environment variable holding the key.
    Env(String),
    /// Path to a file whose contents are the key.
    File(PathBuf),
    /// The raw key inline (throwaway / low-stakes wallets only).
    Raw(String),
}

#[derive(Debug, thiserror::Error)]
#[error("unrecognized key source '{0}' — use op://…, env:VAR, file:/path, or a raw 0x… key")]
pub struct BadKeySource(pub String);

impl FromStr for KeySource {
    type Err = BadKeySource;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.starts_with("op://") {
            Ok(KeySource::Op(s.to_string()))
        } else if let Some(v) = s.strip_prefix("env:") {
            Ok(KeySource::Env(v.to_string()))
        } else if let Some(p) = s.strip_prefix("file:") {
            Ok(KeySource::File(PathBuf::from(p)))
        } else if s.starts_with("0x") {
            Ok(KeySource::Raw(s.to_string()))
        } else {
            Err(BadKeySource(s.to_string()))
        }
    }
}

impl<'de> Deserialize<'de> for KeySource {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Wallet used when none is named (reserved for future ergonomics).
    pub default_wallet: Option<String>,
    #[serde(default)]
    pub wallets: BTreeMap<String, Wallet>,
}

/// A named wallet: a typed key source and an optional default spend ceiling.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Wallet {
    pub key: KeySource,
    /// Default per-payment ceiling in decimal USDC.
    pub max: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("reading config {path}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("parsing config {path}")]
    Parse {
        path: String,
        #[source]
        source: toml::de::Error,
    },
    #[error("wallet '{name}' not found in {path}")]
    NoWallet { name: String, path: String },
}

impl Config {
    /// Load the config file, or an empty config if it doesn't exist.
    pub fn load() -> Result<Self, Error> {
        let path = config_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let s = std::fs::read_to_string(&path).map_err(|source| Error::Read {
            path: path.display().to_string(),
            source,
        })?;
        toml::from_str(&s).map_err(|source| Error::Parse {
            path: path.display().to_string(),
            source,
        })
    }

    /// Look up a wallet by name.
    pub fn wallet(&self, name: &str) -> Result<&Wallet, Error> {
        self.wallets.get(name).ok_or_else(|| Error::NoWallet {
            name: name.to_string(),
            path: config_path().display().to_string(),
        })
    }
}

/// `$X402_PROXY_CONFIG`, else `$XDG_CONFIG_HOME/x402-proxy/config.toml`, else
/// `~/.config/x402-proxy/config.toml`.
pub fn config_path() -> PathBuf {
    if let Ok(p) = std::env::var("X402_PROXY_CONFIG") {
        return PathBuf::from(p);
    }
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config")
        });
    base.join("x402-proxy").join("config.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_source_variants() {
        assert_eq!(
            "op://V/i/f".parse::<KeySource>().unwrap(),
            KeySource::Op("op://V/i/f".into())
        );
        assert_eq!(
            "env:X402_KEY".parse::<KeySource>().unwrap(),
            KeySource::Env("X402_KEY".into())
        );
        assert_eq!(
            "file:/etc/key".parse::<KeySource>().unwrap(),
            KeySource::File("/etc/key".into())
        );
        assert_eq!(
            "0xabc".parse::<KeySource>().unwrap(),
            KeySource::Raw("0xabc".into())
        );
        assert!("wallet:main".parse::<KeySource>().is_err());
        assert!("garbage".parse::<KeySource>().is_err());
    }

    #[test]
    fn parses_typed_wallets() {
        let c: Config = toml::from_str(
            r#"
            default_wallet = "main"
            [wallets.main]
            key = "op://V/i/f"
            max = "0.50"
            [wallets.dev]
            key = "env:X402_DEV_KEY"
            "#,
        )
        .unwrap();
        assert_eq!(c.default_wallet.as_deref(), Some("main"));
        assert_eq!(
            c.wallet("main").unwrap().key,
            KeySource::Op("op://V/i/f".into())
        );
        assert_eq!(c.wallet("main").unwrap().max.as_deref(), Some("0.50"));
        assert_eq!(
            c.wallet("dev").unwrap().key,
            KeySource::Env("X402_DEV_KEY".into())
        );
        assert!(c.wallet("missing").is_err());
    }

    #[test]
    fn rejects_unknown_fields_and_bad_source() {
        assert!(toml::from_str::<Config>("[wallets.x]\nkey='0x1'\nbogus=1").is_err());
        assert!(toml::from_str::<Config>("[wallets.x]\nkey='nonsense'").is_err());
    }
}
