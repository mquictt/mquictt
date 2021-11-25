use std::{fs, path::{Path, PathBuf}, str::FromStr, sync::Arc};

use serde::Deserialize;

use crate::Error;

/// The configuration to launching either server or client. Can create an empty config using
/// [`empty()`] or provide path to a config file in JSON format to read from using [`read()`].
///
/// ### Example
/// ```no_run
/// {
///     "auth": {
/// 	    "ca_cert_file": "certs/rootca.crt",
/// 	    "cert_file": "certs/server/cert.crt",
/// 	    "key_file": "certs/server/cert.key"
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
    pub logs: Option<LogConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Auth {
    pub ca_cert_file: PathBuf,
    pub cert_file: PathBuf,
    pub key_file: PathBuf,
}



macro_rules! __define_log_config {
    ($($field:ident: $field_ty:ty => $default:expr,)+) => {paste::paste! {
        #[derive(Debug, Clone, Deserialize)]
        pub struct LogConfig {
           $(
               #[serde(default = "default_"$field)]
                pub $field: $field_ty,
            )*
        }

        $(
            #[inline]
            fn [<default_ $field>]() -> $field_ty {
                $default
            }
        )*

        impl Default for LogConfig {
            fn default() -> Self {
                LogConfig {
                    $($field: [<default_ $field>](),)*
                }
            }
        }
    }}
}

__define_log_config!(
    path: PathBuf => PathBuf::from("default.mquictt".to_string()),
    cache_capacity: u64 => 1024 * 1024 * 1024, // 1gb
    use_compression: bool => false,
    compression_factor: i32 => 5,
    flush_every_ms: u64 => 500,
);

impl Config {
    /// Create an empty config with no authentication, and logs database in a folder named
    /// `default.mquictt`.
    pub fn empty() -> Arc<Self> {
        Arc::new(Config {
            auth: None,
            logs: Some(LogConfig::default()),
        })
    }

    /// Read the config in JSON format from the given location.
    ///
    /// ### Example
    /// ```no_run
    /// {
    ///     "auth": {
    /// 	    "ca_cert_file": "certs/rootca.crt",
    /// 	    "cert_file": "certs/server/cert.crt",
    /// 	    "key_file": "certs/server/cert.key"
    ///     },
    ///     "logs": {
    ///
    ///     }
    /// }
    /// ```
    pub fn read(path: &impl AsRef<Path>) -> Result<Arc<Self>, Error> {
        Ok(Arc::new(serde_json::from_slice(&fs::read(path)?)?))
    }
}
