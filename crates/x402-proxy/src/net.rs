//! Validated network endpoints — parse-don't-validate at the boundary.
//!
//! [`HttpUrl`] parses and checks a URL once (at CLI-arg parse time, via
//! `FromStr`), so every downstream consumer receives a guaranteed absolute
//! http(s) endpoint and never re-validates or handles a malformed string.

use std::fmt;
use std::str::FromStr;

/// An absolute `http`/`https` URL, validated at construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpUrl(url::Url);

#[derive(Debug, thiserror::Error)]
pub enum HttpUrlError {
    #[error("not a valid URL: {0}")]
    Malformed(#[from] url::ParseError),
    #[error("URL scheme must be http or https, got '{0}'")]
    NotHttp(String),
}

impl HttpUrl {
    /// The underlying parsed URL.
    pub fn as_url(&self) -> &url::Url {
        &self.0
    }

    /// The URL as a string slice (scheme, host, path, query all intact).
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Consume into the underlying [`url::Url`].
    pub fn into_url(self) -> url::Url {
        self.0
    }

    /// Return this URL guaranteed to carry the x402 `payment=x402` query param,
    /// so a caller can pass a bare host (`https://mcp.apify.com`). If a
    /// `payment` param is already present it's left untouched.
    pub fn with_payment_param(&self) -> HttpUrl {
        if self.0.query_pairs().any(|(k, _)| k == "payment") {
            return self.clone();
        }
        let mut url = self.0.clone();
        url.query_pairs_mut().append_pair("payment", "x402");
        HttpUrl(url)
    }
}

impl FromStr for HttpUrl {
    type Err = HttpUrlError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let url = url::Url::parse(s)?;
        match url.scheme() {
            "http" | "https" => Ok(Self(url)),
            other => Err(HttpUrlError::NotHttp(other.to_string())),
        }
    }
}

impl fmt::Display for HttpUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_http_and_https() {
        assert!(
            "https://mcp.apify.com?payment=x402"
                .parse::<HttpUrl>()
                .is_ok()
        );
        assert!("http://localhost:8080/mcp".parse::<HttpUrl>().is_ok());
    }

    #[test]
    fn preserves_query_string() {
        let u: HttpUrl = "https://mcp.apify.com?payment=x402".parse().unwrap();
        assert_eq!(u.as_str(), "https://mcp.apify.com/?payment=x402");
        assert_eq!(u.as_url().query(), Some("payment=x402"));
    }

    #[test]
    fn rejects_non_http_scheme() {
        assert!(matches!(
            "file:///etc/passwd".parse::<HttpUrl>(),
            Err(HttpUrlError::NotHttp(_))
        ));
        assert!(matches!(
            "ftp://example.com".parse::<HttpUrl>(),
            Err(HttpUrlError::NotHttp(_))
        ));
    }

    #[test]
    fn rejects_malformed() {
        assert!(matches!(
            "not a url".parse::<HttpUrl>(),
            Err(HttpUrlError::Malformed(_))
        ));
        // http scheme with no host
        assert!("http://".parse::<HttpUrl>().is_err());
    }

    #[test]
    fn appends_payment_param_when_absent() {
        let bare: HttpUrl = "https://mcp.apify.com".parse().unwrap();
        assert_eq!(
            bare.with_payment_param().as_url().query(),
            Some("payment=x402")
        );
        // already present -> untouched
        let has: HttpUrl = "https://x.test?payment=x402&a=1".parse().unwrap();
        assert_eq!(has.with_payment_param().as_str(), has.as_str());
    }
}
