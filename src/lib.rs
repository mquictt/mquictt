use std::net::SocketAddr;

use futures::StreamExt;

mod client;
mod error;
pub use error::Error;
pub use client::Client;

pub(crate) struct Connection {
    conn: quinn::Connection,
    streams: quinn::IncomingBiStreams,
}

pub(crate) struct QuicServer {
    incoming: quinn::Incoming,
}

impl QuicServer {
    pub(crate) fn new(addr: &SocketAddr) -> Result<Self, Error> {
        let (_, incoming) = quinn::Endpoint::builder().bind(addr)?;
        Ok(QuicServer { incoming })
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
}

#[allow(dead_code)]
pub(crate) type QuicClient = Connection;

impl Connection {
    pub(crate) async fn connect(
        bind_addr: &SocketAddr,
        connect_addr: &SocketAddr,
        server_name: &str,
    ) -> Result<Self, Error> {
        let (endpoint, _) = quinn::Endpoint::builder().bind(bind_addr)?;
        let quinn::NewConnection {
            connection: conn,
            bi_streams: streams,
            ..
        } = endpoint.connect(connect_addr, server_name)?.await?;
        Ok(Connection { conn, streams })
    }

    pub(crate) async fn create_stream(&mut self) -> Result<(quinn::SendStream, quinn::RecvStream), Error> {
        Ok(self.conn.open_bi().await?)
    }

    pub(crate) async fn accept(&mut self) -> Result<(quinn::SendStream, quinn::RecvStream), Error> {
        match self.streams.next().await {
            Some(s) => Ok(s?),
            None => Err(Error::ConnectionBroken),
        }
    }
}
