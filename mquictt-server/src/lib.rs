use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex, RwLock},
};

use bytes::BytesMut;
use log::*;
use mqttbytes::v4;

use mquictt_core::{recv_stream_read, Connection, Publish, QuicServer};
pub use mquictt_core::{Config, Error};

type DataTx = tokio::sync::broadcast::Sender<Publish>;
type DataRx = tokio::sync::broadcast::Receiver<Publish>;

#[derive(Clone)]
struct Transmitter(Arc<RwLock<DataTx>>);

struct Map {
    map: HashMap<String, Transmitter>,
}

impl Map {
    fn new() -> Self {
        Map {
            map: HashMap::new(),
        }
    }

    fn get_or_create_sender(&mut self, topic: &String) -> Result<Transmitter, Error> {
        let (data_tx, _) = tokio::sync::broadcast::channel(1024);
        let data_tx = Transmitter(Arc::new(RwLock::new(data_tx)));
        let rx = self.map.entry(topic.clone()).or_insert(data_tx).clone();
        Ok(rx)
    }
}

type Mapper = Arc<Mutex<Map>>;

/// Spawns a new server that listens for incoming MQTT connects from clients at the given address.
///
/// ```
/// tokio::spawn(server([127, 0, 0, 1], 1883).into(), mquictt::Config::empty());
/// ```
pub async fn server(addr: &SocketAddr, config: Arc<Config>) -> Result<(), Error> {
    let mut listener = QuicServer::new(config, addr)?;
    info!("QUIC server launched at {}", listener.local_addr());
    let map: Mapper = Arc::new(Mutex::new(Map::new()));

    loop {
        let conn = match listener.accept().await {
            Ok(conn) => conn,
            Err(Error::ConnectionBroken) => {
                log::warn!("conn broker {}", listener.local_addr());
                return Ok(());
            }
            e => e?,
        };

        info!("accepted conn from {}", conn.remote_addr());
        let map = map.clone();
        tokio::spawn(connection_handler(conn, map));
    }
}

async fn connection_handler(mut conn: Connection, map: Mapper) -> Result<(), Error> {
    let remote_addr = conn.remote_addr();
    debug!("connection handler spawned for {}", remote_addr);
    let (mut tx, mut rx) = conn.accept_stream().await?;
    debug!("stream accepted for {}", remote_addr);
    let mut buf = BytesMut::with_capacity(2048);

    let mut len = recv_stream_read(&mut rx, &mut buf).await?;
    loop {
        match v4::read(&mut buf, 1024 * 1024) {
            // TODO: check duplicate id
            Ok(v4::Packet::Connect(_)) => break,
            Ok(v4::Packet::Disconnect) => {
                info!("Closing stream from: {}", remote_addr);
                return Ok(());
            }
            Ok(_) => continue,
            Err(mqttbytes::Error::InsufficientBytes(_)) => {
                len += recv_stream_read(&mut rx, &mut buf).await?;
                debug!("recv CONNECT loop len = {}", len);
                continue;
            }
            Err(e) => return Err(Error::MQTT(e)),
        }
    }
    debug!(
        "recved CONNECT packet from {}, len = {}",
        conn.remote_addr(),
        len
    );

    buf.clear();
    if let Err(e) = v4::ConnAck::new(v4::ConnectReturnCode::Success, false).write(&mut buf) {
        return Err(Error::MQTT(e));
    }
    let _write = tx.write_all(&buf).await?;
    debug!("sent CONNACK packet to {}", conn.remote_addr());

    buf.clear();
    let remote_addr = conn.remote_addr();
    loop {
        let (tx, rx) = conn.accept_stream().await?;
        let map = map.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_new_stream(tx, rx, map, remote_addr).await {
                error!("{}", e);
            }
        });

        // tokio::select! {
        //     streams_result = conn.accept() => {
        //         let (tx, rx) = streams_result?;
        //         tokio::spawn(handle_new_stream(tx, rx, mapper.clone()));
        //     }
        // }
    }
}

async fn handle_new_stream(
    tx: quinn::SendStream,
    mut rx: quinn::RecvStream,
    map: Mapper,
    remote_addr: SocketAddr,
) -> Result<(), Error> {
    let mut buf = BytesMut::with_capacity(2048);

    recv_stream_read(&mut rx, &mut buf).await?;
    loop {
        match v4::read(&mut buf, 1024 * 1024) {
            Ok(v4::Packet::Publish(v4::Publish { topic, .. })) => {
                // ignoring first publish's payload as there are no subscribers
                // TODO: handle case when subsribing to topic that is not in mapper
                let data_tx = {
                    let mut map = map.lock().unwrap();
                    map.get_or_create_sender(&topic)?
                };
                debug!("new PUBLISH stream addr = {} id = {}", remote_addr, rx.id());
                return handle_publish(rx, data_tx, buf, remote_addr).await;
            }
            Ok(v4::Packet::Subscribe(v4::Subscribe { filters, .. })) => {
                // only handling a single subsribe for now, as client only sends 1 subscribe at a
                // time
                //
                // TODO: handle multiple subs
                let filter = match filters.get(0) {
                    Some(filter) => filter,
                    None => return Ok(()),
                };
                let data_rx = {
                    let mut map = map.lock().unwrap();
                    let Transmitter(tr) = map.get_or_create_sender(&filter.path)?;
                    let tr = tr.read().unwrap();
                    tr.subscribe()
                };
                debug!(
                    "new SUBSCRIBE stream addr = {} id = {}",
                    remote_addr,
                    rx.id()
                );
                return handle_subscribe(tx, rx, data_rx, remote_addr).await;
            }
            Ok(v4::Packet::Disconnect) => {
                info!("Closing stream from: {}", remote_addr);
                return Ok(());
            }
            Ok(_) => continue,
            Err(mqttbytes::Error::InsufficientBytes(_)) => {
                recv_stream_read(&mut rx, &mut buf).await?;
                continue;
            }
            Err(e) => return Err(Error::MQTT(e)),
        }
    }
}

async fn handle_publish(
    mut rx: quinn::RecvStream,
    data_tx: Transmitter,
    mut buf: BytesMut,
    remote_addr: SocketAddr,
) -> Result<(), Error> {
    buf.reserve(2048);

    debug!(
        "{} :: {} [PS] publish read loop launched, cap = {}",
        remote_addr,
        rx.id(),
        buf.capacity()
    );

    'outer: loop {
        let len = recv_stream_read(&mut rx, &mut buf).await?;
        debug!("{} :: {} [PS] read {} bytes", remote_addr, rx.id(), len);

        loop {
            let publish = match Publish::read(&mut buf) {
                Ok(publish) => publish,
                Err(mqttbytes::Error::InsufficientBytes(_)) => continue 'outer,
                Err(e) => return Err(Error::MQTT(e)),
            };

            let Transmitter(tx) = data_tx.clone();
            {
                let data_tx = tx.write().unwrap();
                data_tx.send(publish)?;
            }

            buf.reserve(2048);
        }
    }
}

async fn handle_subscribe(
    mut tx: quinn::SendStream,
    mut rx: quinn::RecvStream,
    mut data_rx: DataRx,
    remote_addr: SocketAddr,
) -> Result<(), Error> {
    let mut recv_buf = BytesMut::with_capacity(2048);
    let mut send_buf = BytesMut::with_capacity(2048);

    if let Err(e) = v4::SubAck::new(
        0,
        vec![v4::SubscribeReasonCode::Success(mqttbytes::QoS::AtMostOnce)],
    )
    .write(&mut send_buf)
    {
        return Err(Error::MQTT(e));
    }
    tx.write_all(&send_buf).await?;
    send_buf.clear();
    debug!("{} :: {} [SS] SUBACK sent", remote_addr, rx.id());

    loop {
        tokio::select! {
            data_res = data_rx.recv() => {
                let data = data_res?;
                // TODO: use try_recv in loop for buffering
                debug!("{} :: {} [SS] recved pub len = {}", remote_addr, rx.id(), send_buf.len());
                tx.write_all(&data.0).await?;
            }

            read = recv_stream_read(&mut rx, &mut recv_buf) => {
                let len = read?;
                debug!("{} :: {} [SS] read {} bytes", remote_addr, rx.id(), len);
                match v4::read(&mut recv_buf, 1024 * 1024) {
                    Ok(v4::Packet::Unsubscribe(_)) => break,
                    Ok(_) | Err(mqttbytes::Error::InsufficientBytes(_)) => continue,
                    Err(e) => return Err(Error::MQTT(e)),
                };
            }
        }
    }

    Ok(())
}
