use crate::HeaderValue;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::str::FromStr;

/// Additional server options.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "clap", derive(clap::Parser), clap(next_help_heading = "Server options"))]
pub struct ServerConfig {
    /// The cors `allow_origin` header
    #[cfg_attr(
        feature = "clap",
        clap(
            long,
            help = "Set the CORS allow_origin",
            default_value = "*",
            name = "allow-origin",
            value_name = "ALLOW_ORIGIN"
        )
    )]
    pub allow_origin: HeaderValueWrapper,
    /// Whether to enable CORS
    #[cfg_attr(
        feature = "clap",
        clap(long, help = "Disable CORS", conflicts_with = "allow-origin")
    )]
    pub no_cors: bool,
}

// === impl ServerConfig ===

impl ServerConfig {
    /// Sets the "allow origin" header for cors
    pub fn with_allow_origin(mut self, allow_origin: impl Into<HeaderValueWrapper>) -> Self {
        self.allow_origin = allow_origin.into();
        self
    }

    /// Whether to enable CORS
    pub fn set_cors(mut self, cors: bool) -> Self {
        self.no_cors = cors;
        self
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self { allow_origin: "*".parse::<HeaderValue>().unwrap().into(), no_cors: false }
    }
}

#[derive(Clone, Debug)]
pub struct HeaderValueWrapper(pub HeaderValue);

impl FromStr for HeaderValueWrapper {
    type Err = <HeaderValue as FromStr>::Err;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(HeaderValueWrapper(s.parse()?))
    }
}

impl Serialize for HeaderValueWrapper {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.0.to_str().map_err(serde::ser::Error::custom)?)
    }
}

impl<'de> Deserialize<'de> for HeaderValueWrapper {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(Self(s.parse().map_err(serde::de::Error::custom)?))
    }
}

impl std::ops::Deref for HeaderValueWrapper {
    type Target = HeaderValue;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<HeaderValueWrapper> for HeaderValue {
    fn from(wrapper: HeaderValueWrapper) -> Self {
        wrapper.0
    }
}

impl From<HeaderValue> for HeaderValueWrapper {
    fn from(header: HeaderValue) -> Self {
        HeaderValueWrapper(header)
    }
}
