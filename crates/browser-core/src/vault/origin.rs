use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum OriginError {
    #[error("not a valid URL: {0}")]
    InvalidUrl(#[from] url::ParseError),
    #[error("credentials are only kept for http and https, not `{0}`")]
    UnsupportedScheme(String),
    #[error("URL has no host")]
    MissingHost,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Scheme {
    Http,
    Https,
}

impl Scheme {
    pub fn as_str(self) -> &'static str {
        match self {
            Scheme::Http => "http",
            Scheme::Https => "https",
        }
    }
}

/// A web origin as the browser enforces it: scheme, host and port must all
/// match exactly. `accounts.example.com` and `example.com` are different
/// origins, and so are `http://` and `https://` of the same host.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Origin {
    scheme: Scheme,
    host: String,
    port: u16,
}

impl Origin {
    pub fn parse(url: &str) -> Result<Self, OriginError> {
        Self::from_url(&Url::parse(url)?)
    }

    /// Host comes from the URL parser, so it is already lowercased, IDNA
    /// (punycode) encoded and stripped of userinfo; `user@evil.test` tricks
    /// cannot leak into the host.
    pub fn from_url(url: &Url) -> Result<Self, OriginError> {
        let scheme = match url.scheme() {
            "http" => Scheme::Http,
            "https" => Scheme::Https,
            other => return Err(OriginError::UnsupportedScheme(other.to_owned())),
        };
        let host = url
            .host_str()
            .filter(|h| !h.is_empty())
            .ok_or(OriginError::MissingHost)?;
        let port = url
            .port_or_known_default()
            .ok_or(OriginError::MissingHost)?;
        Ok(Self {
            scheme,
            host: host.to_owned(),
            port,
        })
    }

    pub fn scheme(&self) -> Scheme {
        self.scheme
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn is_secure(&self) -> bool {
        self.scheme == Scheme::Https
    }

    /// `scheme://host:port` with the port always written, so the same origin
    /// always produces the same bytes for AAD.
    pub fn canonical(&self) -> String {
        format!("{}://{}:{}", self.scheme.as_str(), self.host, self.port)
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.canonical())
    }
}

impl fmt::Debug for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Origin({})", self.canonical())
    }
}

impl TryFrom<String> for Origin {
    type Error = OriginError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::parse(&text)
    }
}

impl From<Origin> for String {
    fn from(origin: Origin) -> Self {
        origin.canonical()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin(url: &str) -> Origin {
        match Origin::parse(url) {
            Ok(o) => o,
            Err(e) => panic!("{url}: {e}"),
        }
    }

    #[test]
    fn default_ports_are_explicit() {
        assert_eq!(
            origin("https://Example.COM/login?x=1").canonical(),
            "https://example.com:443"
        );
        assert_eq!(
            origin("http://example.com").canonical(),
            "http://example.com:80"
        );
        assert_eq!(origin("https://example.com:8443/").port(), 8443);
    }

    #[test]
    fn exact_match_only() {
        let site = origin("https://example.com/");
        assert_eq!(site, origin("https://example.com:443/other/path"));
        assert_ne!(site, origin("http://example.com/"));
        assert_ne!(site, origin("https://accounts.example.com/"));
        assert_ne!(site, origin("https://example.com.evil.test/"));
        assert_ne!(site, origin("https://example.com:444/"));
    }

    #[test]
    fn userinfo_does_not_spoof_host() {
        assert_eq!(origin("https://example.com@evil.test/").host(), "evil.test");
    }

    #[test]
    fn idn_is_punycoded() {
        assert_eq!(
            origin("https://bücher.example/").host(),
            "xn--bcher-kva.example"
        );
    }

    #[test]
    fn rejects_non_web_schemes() {
        assert!(matches!(
            Origin::parse("file:///C:/x"),
            Err(OriginError::UnsupportedScheme(_))
        ));
        assert!(matches!(
            Origin::parse("data:text/html,hi"),
            Err(OriginError::UnsupportedScheme(_))
        ));
        assert!(Origin::parse("not a url").is_err());
    }

    #[test]
    fn round_trips_canonical_form() {
        let site = origin("https://[::1]:3000/");
        assert_eq!(Origin::parse(&site.canonical()), Ok(site));
    }
}
