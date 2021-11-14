use std::{fs, path::Path, sync::Arc};

use serde::Deserialize;

use crate::Error;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub auth: Option<Auth>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Auth {
    pub ca_cert_file: String,
    pub cert_file: String,
    pub key_file: String,
}

impl Config {
    pub fn empty() -> Arc<Self> {
        Arc::new(Config { auth: None })
    }

    pub fn read(path: &impl AsRef<Path>) -> Result<Arc<Self>, Error> {
        Ok(Arc::new(serde_json::from_slice(&fs::read(path)?)?))
    }
}
