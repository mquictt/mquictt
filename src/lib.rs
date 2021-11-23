#![doc = include_str!("../README.md")]

use std::{fs::File, io::BufReader, net::SocketAddr, sync::Arc};

use bytes::{BufMut, BytesMut};
use config::Auth;
use futures::StreamExt;
#[allow(unused_imports)]
use log::*;
use quinn::{ClientConfig, ServerConfig, TransportConfig};
use rustls::{AllowAnyAnonymousOrAuthenticatedClient, Certificate, PrivateKey, RootCertStore};

mod client;
mod config;
mod error;
mod protocol;
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
    #[allow(dead_code)]
    config: Arc<Config>,
    incoming: quinn::Incoming,
    endpoint: quinn::Endpoint,
}

impl QuicServer {
    pub(crate) fn new(config: Arc<Config>, addr: &SocketAddr) -> Result<Self, Error> {
        let mut builder = quinn::Endpoint::builder();
        builder.listen(server_config(&config)?);
        let (endpoint, incoming) = builder.bind(addr)?;
        Ok(QuicServer {
            config,
            incoming,
            endpoint,
        })
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

        let quinn::NewConnection {
            connection: conn,
            bi_streams: streams,
            ..
        } = endpoint.connect(connect_addr, server_name)?.await?;
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
            let keys = rustls_pemfile::rsa_private_keys(&mut BufReader::new(key_file))?;

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
            store
                .add_pem_file(ca_file)
                .map_err(|_| Error::MissingCertificate)?;

            let mut client_config = rustls::ClientConfig::default();
            client_config.root_store = store;
            client_config.set_single_client_cert(certs, key)?;
            client_config.versions = vec![rustls::ProtocolVersion::TLSv1_3];

            Ok(ClientConfig {
                transport: Arc::new(TransportConfig::default()),
                crypto: Arc::new(client_config),
            })
        }

        None => Ok(ClientConfig::default()),
    }
}

fn server_config(config: &Arc<Config>) -> Result<ServerConfig, Error> {
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
            let keys = rustls_pemfile::rsa_private_keys(&mut BufReader::new(key_file))?;

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
            store
                .add_pem_file(ca_file)
                .map_err(|_| Error::MissingCertificate)?;

            let mut config =
                rustls::ServerConfig::new(AllowAnyAnonymousOrAuthenticatedClient::new(store));
            config.set_single_cert(certs, key)?;
            config.versions = vec![rustls::ProtocolVersion::TLSv1_3];

            let mut server_config = ServerConfig::default();
            server_config.crypto = Arc::new(config);

            Ok(server_config)
        }

        None => Ok(ServerConfig::default()),
    }
}

#[inline(always)]
pub(crate) async fn recv_stream_read(
    rx: &mut quinn::RecvStream,
    buf: &mut BytesMut,
) -> Result<usize, Error> {
    // SAFETY: we trust qiunn's implementation to not misuse the array, and we advance the
    // `BytesMut`'s cursor to proper length as well
    let dst = unsafe { bytesmut_as_arr(buf) };
    let len = match rx.read(dst).await? {
        Some(len) => {
            unsafe { buf.advance_mut(len) };
            Ok(len)
        }
        None => Err(Error::ConnectionBroken),
    };
}

/// Reads from [`quinn::RecvStream`] into a [`BytesMut`].
///
/// Converts the [`BytesMut`] to `&mut [u8]`, with the length of returned array being equal to the
/// remaining capacity of the `BytesMut` as determined by [`bytes::BufMut::has_remaining_mut()`].
///
/// `BytesMut` conversion part copied from [tokio's implementation] of [`TcpStream::read_buf()]`.
///
/// [tokio's implementation]: https://docs.rs/tokio/1.14.0/src/tokio/net/tcp/stream.rs.html#720-722
#[inline(always)]
pub(crate) unsafe fn bytesmut_as_arr(buf: &mut BytesMut) -> &mut [u8] {
    &mut *(buf.chunk_mut() as *mut _ as *mut [std::mem::MaybeUninit<u8>] as *mut [u8])
}
