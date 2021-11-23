use std::{net::SocketAddr, sync::Arc};

use bytes::{Bytes, BytesMut};
use log::*;
use mqttbytes::v4;

use crate::{recv_stream_read, Config, Connection, Error};

/// Used to initiate connection to a MQTT server over QUIC.
///
/// Can be initiated using [`connect()`] function.
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
///
/// [`connect()`]: `Client::connect`
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
        info!("connected to {}", conn.remote_addr());
        // stream to send the mqtt connect packet
        let (mut tx, mut rx) = conn.create_stream().await?;
        let mut buf = BytesMut::with_capacity(2048);

        if let Err(e) = v4::Connect::new(id).write(&mut buf) {
            return Err(Error::MQTT(e));
        }
        let write = tx.write(&buf).await?;
        debug!("sent CONNECT to {}, len = {}", conn.remote_addr(), write);

        buf.clear();
        recv_stream_read(&mut rx, &mut buf).await?;
        loop {
            match v4::read(&mut buf, 1024 * 1024) {
                Ok(v4::Packet::ConnAck(_)) => break,
                Ok(_) => continue,
                // read more bytes in case current bytes not enough
                Err(mqttbytes::Error::InsufficientBytes(_)) => {
                    recv_stream_read(&mut rx, &mut buf).await?;
                    continue;
                }
                Err(e) => return Err(Error::MQTT(e)),
            }
        }
        debug!("recved CONNACK from {}", conn.remote_addr());

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
        // create a new stream
        let (mut tx, _) = self.conn.create_stream().await?;
        let topic = topic.into();

        // send publish with `init_payload`
        let mut buf = BytesMut::new();
        if let Err(e) = v4::Publish::from_bytes(&topic, mqttbytes::QoS::AtMostOnce, init_payload)
            .write(&mut buf)
        {
            return Err(Error::MQTT(e));
        }
        tx.write_all(&buf).await?;
        debug!("flushed {} bytes", buf.len());
        buf.clear();

        // not waiting for puback as QoS0 at MQTT level

        Ok(Publisher {
            topic,
            tx,
            buf: BytesMut::new(),
        })
    }

    /// Creates a new subscribe stream for the given topic from which publishes messages can be
    /// read. See [`Subscriber`] for more details.
    pub async fn subscriber(&mut self, topic: impl Into<String>) -> Result<Subscriber, Error> {
        // create a new stream
        let (mut tx, mut rx) = self.conn.create_stream().await?;
        let mut buf = BytesMut::new();

        // send subscribe packet
        if let Err(e) = v4::Subscribe::new(topic, mqttbytes::QoS::AtMostOnce).write(&mut buf) {
            return Err(Error::MQTT(e));
        }
        let _write = tx.write_all(&buf).await?;
        debug!("sent SUBSCRIBE to {}", self.conn.remote_addr());

        buf.clear();
        // read suback
        recv_stream_read(&mut rx, &mut buf).await?;
        loop {
            match v4::read(&mut buf, 1024 * 1024) {
                Ok(v4::Packet::SubAck(_)) => break,
                Ok(_) => continue,
                Err(mqttbytes::Error::InsufficientBytes(_)) => {
                    recv_stream_read(&mut rx, &mut buf).await?;
                    continue;
                }
                Err(e) => return Err(Error::MQTT(e)),
            }
        }
        debug!("recved SUBACK from {}", self.conn.remote_addr());

        // same buffer transfered as it might contain data for some publishes
        Ok(Subscriber { rx, tx, buf })
    }
}

/// A publish stream for a topic. The stream is closed when this gets dropped. All messages
/// published using a single `Publisher` happen to the same topic.
///
/// Note that callers explcitly need to call [`flush()`] to flush all the publishes to network.
///
/// ```
/// let mut client = mquictt::Client::connect(bind_addr, connect_addr, server_name, id, config).await?;
/// let mut publisher = client.publisher("hello/world", b"hello!".into()).await?;
/// publisher.publish(b"hello again!".into()).await?;
/// publisher.flush().await?;
/// ```
///
/// [`flush()`]: `Publisher::flush`
pub struct Publisher {
    topic: String,
    tx: quinn::SendStream,
    buf: BytesMut,
}

impl Publisher {
    /// Write a publish packet with given payload to the topic stream. Note that [`flush()`]
    /// **needs** to be called explcitly else nothing will be written to the network.
    ///
    /// [`flush()`]: `Publisher::flush`
    pub async fn publish(&mut self, payload: Bytes) -> Result<(), Error> {
        if let Err(e) = v4::Publish::from_bytes(&self.topic, mqttbytes::QoS::AtMostOnce, payload)
            .write(&mut self.buf)
        {
            return Err(Error::MQTT(e));
        }
        Ok(())
    }

    /// Flush all the packets to the stream. This **needs** to be called after calling
    /// [`publish()`] a bunch of times.
    ///
    /// [`publish()`]: `Publisher::publish`
    pub async fn flush(&mut self) -> Result<(), Error> {
        self.tx.write_all(&self.buf).await?;
        debug!("flushed {} bytes", self.buf.len());
        self.buf.clear();
        Ok(())
    }

    /// Topic to which this stream publishes to.
    pub fn topic(&self) -> &str {
        &self.topic
    }
}

/// Subsriber to a single topic.
///
/// Note that callers explcitly need to call [`flush()`] to flush all the publishes to network.
///
/// ```
/// let mut client = mquictt::Client::connect(bind_addr, connect_addr, server_name, id, config).await?;
/// let mut subscriber = client.subscriber("hello/world").await?;
/// let bytes = subscriber.read().await?;
/// let _ = subscriber.close().await;
/// ```
///
/// [`flush()`]: `Publisher::flush`
pub struct Subscriber {
    tx: quinn::SendStream,
    rx: quinn::RecvStream,
    buf: BytesMut,
}

impl Subscriber {
    /// Read a single publish, or wait until one is received.
    pub async fn read(&mut self) -> Result<Bytes, Error> {
        loop {
            match v4::read(&mut self.buf, 1024 * 1024) {
                Ok(v4::Packet::Publish(packet)) => return Ok(packet.payload),
                Ok(_) => continue,
                Err(mqttbytes::Error::InsufficientBytes(_)) => {
                    debug!("sub reading from net");
                    let len = recv_stream_read(&mut self.rx, &mut self.buf).await?;
                    debug!("read {} pub bytes", len);
                    continue;
                }
                Err(e) => return Err(Error::MQTT(e)),
            }
        }
    }

    /// Sends a disconnect packet, closes the entire connection (not just this stream).
    pub async fn close(&mut self) -> Result<(), Error> {
        self.buf.clear();
        if let Err(e) = v4::Disconnect.write(&mut self.buf) {
            return Err(Error::MQTT(e));
        }
        self.tx.write_all(&self.buf).await?;
        Ok(())
    }
}
