use std::{fs::File, io::BufReader, path::Path, sync::Arc};

use quinn::{ClientConfig, ServerConfig, TransportConfig};
use rustls::{AllowAnyAnonymousOrAuthenticatedClient, Certificate, PrivateKey, RootCertStore};
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

impl Auth {
    /// Get certificates from cert file
    pub fn get_certs(&self) -> Result<Vec<Certificate>, Error> {
        let cert_file = File::open(&self.cert_file)?;
        let certs = rustls_pemfile::certs(&mut BufReader::new(cert_file))?;

        let certs = certs
            .iter()
            .map(|cert| Certificate(cert.to_vec()))
            .collect();

        Ok(certs)
    }

    /// Get the private key from key file
    pub fn get_key(&self) -> Result<PrivateKey, Error> {
        let key_file = File::open(&self.key_file)?;
        let keys = rustls_pemfile::rsa_private_keys(&mut BufReader::new(key_file))?;

        let key = match keys.first() {
            Some(k) => k.clone(),
            None => return Err(Error::MissingCertificate),
        };
        Ok(PrivateKey(key))
    }

    /// Get CA root files
    pub fn get_ca(&self) -> Result<RootCertStore, Error> {
        let ca_file = File::open(&self.ca_cert_file)?;
        let ca_file = &mut BufReader::new(ca_file);
        let mut store = RootCertStore::empty();
        store
            .add_pem_file(ca_file)
            .map_err(|_| Error::MissingCertificate)?;

        Ok(store)
    }

    pub fn client_config(&self) -> Result<ClientConfig, Error> {
        let certs = self.get_certs()?;
        let key = self.get_key()?;
        let ca_root = self.get_ca()?;

        let mut client_config = rustls::ClientConfig::default();
        client_config.root_store = ca_root;
        client_config.set_single_client_cert(certs, key)?;
        client_config.versions = vec![rustls::ProtocolVersion::TLSv1_3];

        Ok(ClientConfig {
            transport: Arc::new(TransportConfig::default()),
            crypto: Arc::new(client_config),
        })
    }

    pub fn server_config(&self) -> Result<ServerConfig, Error> {
        let certs = self.get_certs()?;
        let key = self.get_key()?;
        let ca_root = self.get_ca()?;

        let mut config =
            rustls::ServerConfig::new(AllowAnyAnonymousOrAuthenticatedClient::new(ca_root));
        config.set_single_cert(certs, key)?;
        config.versions = vec![rustls::ProtocolVersion::TLSv1_3];

        let mut server_config = ServerConfig::default();
        server_config.crypto = Arc::new(config);

        Ok(server_config)
    }
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
        let file_contents = std::fs::read(path)?;
        let config = serde_json::from_slice(&file_contents)?;
        Ok(Arc::new(config))
    }
}
