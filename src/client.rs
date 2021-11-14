use std::net::SocketAddr;

use bytes::{Bytes, BytesMut};
use mqttbytes::v4;

use crate::{Connection, Error};

pub struct Client {
    conn: Connection,
}

impl Client {
    pub async fn connect(
        bind_addr: &SocketAddr,
        connect_addr: &SocketAddr,
        server_name: &str,
        id: impl Into<String>,
    ) -> Result<Self, Error> {
        let mut conn = Connection::connect(bind_addr, connect_addr, server_name).await?;
        let (mut tx, mut rx) = conn.create_stream().await?;
        let mut buf = BytesMut::new();

        if let Err(e) = v4::Connect::new(id).write(&mut buf) {
            return Err(Error::MQTT(e));
        }
        let _write = tx.write(&buf).await?;

        buf.clear();
        rx.read(&mut buf).await?;
        loop {
            match v4::read(&mut buf, 1024 * 1024) {
                Ok(v4::Packet::ConnAck(_)) => break,
                Ok(_) => continue,
                Err(mqttbytes::Error::InsufficientBytes(_)) => {
                    rx.read(&mut buf).await?;
                    continue;
                }
                Err(e) => return Err(Error::MQTT(e)),
            }
        }

        Ok(Client { conn })
    }

    // `init_payload` needed as we need to let server know what type of stream this is
    pub async fn publisher(&mut self, topic: impl Into<String>, init_payload: Bytes) -> Result<Publisher, Error> {
        let (mut tx, _) = self.conn.create_stream().await?;
        let topic = topic.into();

        let mut buf = BytesMut::new();
        if let Err(e) = v4::Publish::from_bytes(&topic, mqttbytes::QoS::AtMostOnce, init_payload).write(&mut buf) {
            return Err(Error::MQTT(e));
        }
        let _write = tx.write(&buf).await?;

        Ok(Publisher {
            topic,
            tx,
            buf: BytesMut::new(),
        })
    }

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
        Ok(Subscriber {
            rx, tx, buf
        })
    }
}

pub struct Publisher {
    topic: String,
    tx: quinn::SendStream,
    buf: BytesMut
}

impl Publisher {
    pub async fn publish(&mut self, payload: Bytes) -> Result<(), Error> {
        if let Err(e) =
            v4::Publish::from_bytes(&self.topic, mqttbytes::QoS::AtMostOnce, payload).write(&mut self.buf)
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
