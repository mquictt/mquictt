#![doc = include_str!("../../README.md")]

use std::{net::SocketAddr, sync::Arc};

use bytes::{BufMut, BytesMut};
use futures::StreamExt;
use quinn::{ClientConfig, ServerConfig};

mod config;
mod error;
mod protocol;
pub use config::Config;
pub use error::Error;
pub use protocol::Publish;

pub struct Connection {
    conn: quinn::Connection,
    streams: quinn::IncomingBiStreams,
}

pub struct QuicServer {
    #[allow(dead_code)]
    config: Arc<Config>,
    incoming: quinn::Incoming,
    endpoint: quinn::Endpoint,
}

impl QuicServer {
    pub fn new(config: Arc<Config>, addr: &SocketAddr) -> Result<Self, Error> {
        let mut builder = quinn::Endpoint::builder();
        let server_config = match &config.auth {
            Some(auth) => auth.server_config()?,
            None => ServerConfig::default(),
        };
        builder.listen(server_config);
        let (endpoint, incoming) = builder.bind(addr)?;

        Ok(QuicServer {
            config,
            incoming,
            endpoint,
        })
    }

    pub async fn accept(&mut self) -> Result<Connection, Error> {
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

    pub fn local_addr(&self) -> SocketAddr {
        self.endpoint.local_addr().unwrap()
    }
}

#[allow(dead_code)]
pub type QuicClient = Connection;

impl Connection {
    pub async fn connect(
        bind_addr: &SocketAddr,
        connect_addr: &SocketAddr,
        server_name: &str,
        config: Arc<Config>,
    ) -> Result<Self, Error> {
        let mut builder = quinn::Endpoint::builder();
        let client_config = match &config.auth {
            Some(auth) => auth.client_config()?,
            None => ClientConfig::default(),
        };
        builder.default_client_config(client_config);
        let (endpoint, _) = builder.bind(bind_addr)?;

        let quinn::NewConnection {
            connection: conn,
            bi_streams: streams,
            ..
        } = endpoint.connect(connect_addr, server_name)?.await?;

        Ok(Connection { conn, streams })
    }

    pub async fn create_stream(&mut self) -> Result<(quinn::SendStream, quinn::RecvStream), Error> {
        Ok(self.conn.open_bi().await?)
    }

    pub async fn create_send_stream(&mut self) -> Result<quinn::SendStream, Error> {
        Ok(self.conn.open_uni().await?)
    }

    pub async fn accept_stream(&mut self) -> Result<(quinn::SendStream, quinn::RecvStream), Error> {
        match self.streams.next().await {
            Some(s) => Ok(s?),
            None => Err(Error::ConnectionBroken),
        }
    }

    pub fn remote_addr(&self) -> SocketAddr {
        self.conn.remote_address()
    }
}

#[inline(always)]
pub async fn recv_stream_read(
    rx: &mut quinn::RecvStream,
    buf: &mut BytesMut,
) -> Result<usize, Error> {
    let dst = unsafe { bytesmut_as_arr(buf) };
    match rx.read(dst).await? {
        Some(len) => {
            unsafe { buf.advance_mut(len) };
            Ok(len)
        }
        None => Err(Error::ConnectionBroken),
    }
}

/// Reads from [`quinn::RecvStream`] into a [`BytesMut`].
///
/// Converts the [`BytesMut`] to `&mut [u8]`, with the length of returned array being equal to the
/// remaining capacity of the `BytesMut` as determined by [`bytes::BufMut::has_remaining_mut()`].
///
/// `BytesMut` conversion part copied from [tokio's implementation] of [`TcpStream::read_buf()`].
///
/// # Safety
/// we trust qiunn's implementation to not misuse the array, and we advance the `BytesMut`'s cursor
/// to proper length as well
///
/// [tokio's implementation]: https://docs.rs/tokio/1.14.0/src/tokio/net/tcp/stream.rs.html#720-722
#[inline(always)]
pub unsafe fn bytesmut_as_arr(buf: &mut BytesMut) -> &mut [u8] {
    &mut *(buf.chunk_mut() as *mut _ as *mut [std::mem::MaybeUninit<u8>] as *mut [u8])
}
