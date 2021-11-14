use std::{fs::File, io::BufReader, net::SocketAddr, sync::Arc};

use config::Auth;
use futures::StreamExt;
use log::*;
use quinn::{ClientConfig, TransportConfig};
use rustls::{Certificate, PrivateKey, RootCertStore};

mod client;
mod config;
mod error;
mod server;
pub use client::*;
pub use config::Config;
pub use error::Error;
pub use server::*;

pub(crate) struct Connection {
    conn: quinn::Connection,
    streams: quinn::IncomingBiStreams,
}

pub(crate) struct QuicServer {
    config: Arc<Config>,
    incoming: quinn::Incoming,
    endpoint: quinn::Endpoint,
}

impl QuicServer {
    pub(crate) fn new(config: Arc<Config>, addr: &SocketAddr) -> Result<Self, Error> {
        let mut builder = quinn::Endpoint::builder();
        builder.default_client_config(client_config(&config)?);
        let (endpoint, incoming) = builder.bind(addr)?;
        info!("QUIC server launched at {}", endpoint.local_addr().unwrap());
        Ok(QuicServer { config, incoming, endpoint })
    }

    pub(crate) async fn accept(&mut self) -> Result<Connection, Error> {
        let quinn::NewConnection {
            connection: conn,
            bi_streams: streams,
            ..
        } = match self.incoming.next().await {
            Some(connecting) => connecting.await?,
            None => return Err(Error::ConnectionBroken),
        };
        Ok(Connection { conn, streams })
    }

    pub(crate) fn local_addr(&self) -> SocketAddr {
        self.endpoint.local_addr().unwrap()
    }
}

#[allow(dead_code)]
pub(crate) type QuicClient = Connection;

impl Connection {
    pub(crate) async fn connect(
        bind_addr: &SocketAddr,
        connect_addr: &SocketAddr,
        server_name: &str,
        config: Arc<Config>,
    ) -> Result<Self, Error> {
        let mut builder = quinn::Endpoint::builder();
        builder.default_client_config(client_config(&config)?);
        let (endpoint, _) = builder.bind(bind_addr)?;

        info!("connecting to {}", connect_addr);
        let quinn::NewConnection {
            connection: conn,
            bi_streams: streams,
            ..
        } = endpoint.connect(connect_addr, server_name)?.await?;
        info!("connected to {}", conn.remote_address());
        Ok(Connection { conn, streams })
    }

    pub(crate) async fn create_stream(
        &mut self,
    ) -> Result<(quinn::SendStream, quinn::RecvStream), Error> {
        Ok(self.conn.open_bi().await?)
    }

    pub(crate) async fn accept(&mut self) -> Result<(quinn::SendStream, quinn::RecvStream), Error> {
        match self.streams.next().await {
            Some(s) => Ok(s?),
            None => Err(Error::ConnectionBroken),
        }
    }

    pub(crate) fn remote_addr(&self) -> SocketAddr {
        self.conn.remote_address()
    }
}

fn client_config(config: &Arc<Config>) -> Result<ClientConfig, Error> {
    match &config.auth {
        Some(Auth {
            cert_file: cert_path,
            key_file: key_path,
            ca_cert_file: ca_path,
        }) => {
            // Get certificates
            let cert_file = File::open(&cert_path)?;
            let certs = rustls_pemfile::certs(&mut BufReader::new(cert_file))?;

            // Get private key
            let key_file = File::open(&key_path)?;
            let keys = rustls_pemfile::pkcs8_private_keys(&mut BufReader::new(key_file))?;

            // Get the first key
            let key = match keys.first() {
                Some(k) => k.clone(),
                None => return Err(Error::MissingCertificate),
            };

            let certs = certs
                .iter()
                .map(|cert| Certificate(cert.to_vec()))
                .collect();
            let key = PrivateKey(key);

            let ca_file = File::open(ca_path)?;
            let ca_file = &mut BufReader::new(ca_file);
            let mut store = RootCertStore::empty();
            store.add_pem_file(ca_file).map_err(|_| Error::MissingCertificate)?;


            let mut client_config = rustls::ClientConfig::default();
            client_config.root_store = store;
            client_config.set_single_client_cert(certs, key)?;

            Ok(ClientConfig {
                transport: Arc::new(TransportConfig::default()),
                crypto: Arc::new(client_config),
            })
        }

        None => Ok(ClientConfig::default()),
    }
}
