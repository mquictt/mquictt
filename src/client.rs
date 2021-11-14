use std::{net::SocketAddr, sync::Arc};

use bytes::{Bytes, BytesMut};
// use log::*;
use mqttbytes::v4;

use crate::{Config, Connection, Error};

/// Used to initiate connection to a MQTT server over QUIC.
///
/// Can be initiated using [`Client::connect`] function.
/// ```no_run
/// # async fn run(
/// # bind_addr: &SocketAddr,
/// # connect_addr: &SocketAddr,
/// # server_name: &str,
/// # id: impl Into<String>,
/// # config: Arc<Config>,
/// # ) -> Result<(), mquictt::Error> {
/// let mut client = mquictt::Client::connect(bind_addr, connect_addr, server_name, id, config).await?;
/// #}
/// ```
pub struct Client {
    conn: Connection,
}

impl Client {
    /// Connects to the given `connect_addr` by binding to the `bind_addr`.
    pub async fn connect(
        bind_addr: &SocketAddr,
        connect_addr: &SocketAddr,
        server_name: &str,
        id: impl Into<String>,
        config: Arc<Config>,
    ) -> Result<Self, Error> {
        // create a connection
        let mut conn = Connection::connect(bind_addr, connect_addr, server_name, config).await?;
        // stream to send the mqtt connect packet
        let (mut tx, mut rx) = conn.create_stream().await?;
        let mut buf = BytesMut::new();

        if let Err(e) = v4::Connect::new(id).write(&mut buf) {
            return Err(Error::MQTT(e));
        }
        let _write = tx.write(&buf).await?;

        buf.clear();
        // read the connack packet
        rx.read(&mut buf).await?;
        loop {
            match v4::read(&mut buf, 1024 * 1024) {
                Ok(v4::Packet::ConnAck(_)) => break,
                Ok(_) => continue,
                // read more bytes in case current bytes not enough
                Err(mqttbytes::Error::InsufficientBytes(_)) => {
                    rx.read(&mut buf).await?;
                    continue;
                }
                Err(e) => return Err(Error::MQTT(e)),
            }
        }

        Ok(Client { conn })
    }

    /// Creates a new publish stream for the given topic. Further publishes will be on the same
    /// topic. See [`Publisher`] for more details.
    // `init_payload` needed as we need to let server know what type of stream this is
    pub async fn publisher(
        &mut self,
        topic: impl Into<String>,
        init_payload: Bytes,
    ) -> Result<Publisher, Error> {
        let (mut tx, _) = self.conn.create_stream().await?;
        let topic = topic.into();

        let mut buf = BytesMut::new();
        if let Err(e) = v4::Publish::from_bytes(&topic, mqttbytes::QoS::AtMostOnce, init_payload)
            .write(&mut buf)
        {
            return Err(Error::MQTT(e));
        }
        let _write = tx.write_all(&buf).await?;

        Ok(Publisher {
            topic,
            tx,
            buf: BytesMut::new(),
        })
    }

    /// Creates a new subscribe stream for the given topic from which publishes messages can be
    /// read. See [`Subscriber`] for more details.
    pub async fn subsriber(&mut self, topic: impl Into<String>) -> Result<Subscriber, Error> {
        let (mut tx, mut rx) = self.conn.create_stream().await?;
        let mut buf = BytesMut::new();

        if let Err(e) = v4::Subscribe::new(topic, mqttbytes::QoS::AtMostOnce).write(&mut buf) {
            return Err(Error::MQTT(e));
        }
        let _write = tx.write(&buf).await?;

        buf.clear();
        rx.read(&mut buf).await?;
        loop {
            match v4::read(&mut buf, 1024 * 1024) {
                Ok(v4::Packet::SubAck(_)) => break,
                Ok(_) => continue,
                Err(mqttbytes::Error::InsufficientBytes(_)) => {
                    rx.read(&mut buf).await?;
                    continue;
                }
                Err(e) => return Err(Error::MQTT(e)),
            }
        }

        buf.clear();
        Ok(Subscriber { rx, tx, buf })
    }
}

/// A publish stream for a topic. The stream is closed when this gets dropped. All messages
/// published using a single `Publisher` happen to the same topic.
///
/// ```
/// let mut client = mquictt::Client::connect(bind_addr, connect_addr, server_name, id, config).await?;
/// let mut publisher = client.publisher("hello/world", b"hello!".into()).await?;
/// publisher.flush().await?;
/// ```
pub struct Publisher {
    topic: String,
    tx: quinn::SendStream,
    buf: BytesMut,
}

impl Publisher {
    pub async fn publish(&mut self, payload: Bytes) -> Result<(), Error> {
        if let Err(e) = v4::Publish::from_bytes(&self.topic, mqttbytes::QoS::AtMostOnce, payload)
            .write(&mut self.buf)
        {
            return Err(Error::MQTT(e));
        }
        Ok(())
    }

    pub async fn flush(&mut self) -> Result<(), Error> {
        self.tx.write_all(&self.buf).await?;
        self.buf.clear();
        Ok(())
    }

    pub fn topic(&self) -> &str {
        &self.topic
    }
}

pub struct Subscriber {
    tx: quinn::SendStream,
    rx: quinn::RecvStream,
    buf: BytesMut,
}

impl Subscriber {
    pub async fn read(&mut self) -> Result<Bytes, Error> {
        loop {
            match v4::read(&mut self.buf, 1024 * 1024) {
                Ok(v4::Packet::Publish(packet)) => return Ok(packet.payload),
                Ok(_) => continue,
                Err(mqttbytes::Error::InsufficientBytes(_)) => {
                    self.rx.read(&mut self.buf).await?;
                    continue;
                }
                Err(e) => return Err(Error::MQTT(e)),
            }
        }
    }

    pub async fn close(&mut self) -> Result<(), Error> {
        self.buf.clear();
        if let Err(e) = v4::Disconnect.write(&mut self.buf) {
            return Err(Error::MQTT(e));
        }
        self.tx.write_all(&self.buf).await?;
        Ok(())
    }
}
