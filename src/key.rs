//! Secret resolution — op:// refs to in-memory key material.
//!
//! The resolved value lives in `Zeroizing<String>` and must never be logged,
//! written to disk, or placed in argv. The ref itself is also kept out of
//! logs (op:// refs leak vault structure).

use zeroize::{Zeroize, Zeroizing};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to run `op` — is the 1Password CLI installed and on PATH?")]
    Spawn(#[source] std::io::Error),
    #[error("`op read` failed ({status}): {stderr}")]
    OpFailed { status: String, stderr: String },
    #[error("`op read` returned empty output")]
    Empty,
}

/// Port: resolve a secret reference to in-memory key material.
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
        Self { program: program.to_string() }
    }
}

impl Default for OpCli {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretResolver for OpCli {
    fn resolve(&self, secret_ref: &str) -> Result<Zeroizing<String>, Error> {
        let output = std::process::Command::new(&self.program)
            .arg("read")
            .arg(secret_ref)
            .output()
            .map_err(Error::Spawn)?;
        if !output.status.success() {
            return Err(Error::OpFailed {
                status: output.status.to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }
        let mut raw = Zeroizing::new(
            String::from_utf8_lossy(&output.stdout).into_owned(),
        );
        let trimmed = Zeroizing::new(raw.trim().to_string());
        raw.zeroize();
        if trimmed.is_empty() {
            return Err(Error::Empty);
        }
        Ok(trimmed)
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
}
