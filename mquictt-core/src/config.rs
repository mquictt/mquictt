use std::{fs, path::Path, sync::Arc};

use serde::Deserialize;

use crate::Error;

/// The configuration to launching either server or client. Can create an empty config using
/// [`empty()`] or provide path to a config file in JSON format to read from using [`read()`].
///
/// ### Example
/// ```no_run
/// {
///     "auth": {
///         "ca_cert_file": "certs/rootca.crt",
///         "cert_file": "certs/server/cert.crt",
///         "key_file": "certs/server/cert.key"
///     }
/// }
/// ```
///
/// [`empty()`]: Config::empty
/// [`read()`]: Config::read
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// paths to files needed for authentication.
    pub auth: Option<Auth>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Auth {
    pub ca_cert_file: String,
    pub cert_file: String,
    pub key_file: String,
}

impl Config {
    /// Create an empty config with no authentication.
    pub fn empty() -> Arc<Self> {
        Arc::new(Config { auth: None })
    }

    /// Read the config in JSON format from the given location.
    ///
    /// ### Example
    /// ```no_run
    /// {
    ///     "auth": {
    ///         "ca_cert_file": "certs/rootca.crt",
    ///         "cert_file": "certs/server/cert.crt",
    ///         "key_file": "certs/server/cert.key"
    ///     }
    /// }
    /// ```
    pub fn read(path: &impl AsRef<Path>) -> Result<Arc<Self>, Error> {
        Ok(Arc::new(serde_json::from_slice(&fs::read(path)?)?))
    }
}
