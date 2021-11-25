use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use bytes::BytesMut;
use futures::AsyncWriteExt;
use log::*;
use mqttbytes::v4;

use mquictt_core::{recv_stream_read, Connection, MQTTRead, QuicServer};
pub use mquictt_core::{Config, Error};

type NotifTx = flume::Sender<()>;
type NotifRx = flume::Receiver<()>;

type Mapper = Arc<Mutex<HashMap<String, RoutingConfig>>>;

#[derive(Debug, Clone)]
struct RoutingConfig {
    rx: NotifRx,
    tx: NotifTx,
    db: sled::Db,
}

/// Spawns a new server that listens for incoming MQTT connects from clients at the given address.
///
/// ```
/// tokio::spawn(server([127, 0, 0, 1], 1883).into(), mquictt::Config::empty());
/// ```
pub async fn server(addr: &SocketAddr, config: Arc<Config>) -> Result<(), Error> {
    let mut listener = QuicServer::new(config.clone(), addr)?;
    info!("QUIC server launched at {}", listener.local_addr());
    let mapper: Mapper = Arc::new(Mutex::new(HashMap::default()));
    let conns_list = Arc::new(Mutex::new(HashSet::default()));

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
        tokio::spawn(connection_handler(
            conn,
            conns_list.clone(),
            mapper.clone(),
            config.clone(),
        ));
    }
}

async fn connection_handler(
    mut conn: Connection,
    conns_list: Arc<Mutex<HashSet<String>>>,
    mapper: Mapper,
    config: Arc<Config>,
) -> Result<(), Error> {
    debug!("connection handler spawned for {}", conn.remote_addr());
    let (mut tx, mut rx) = conn.accept().await?;
    debug!("stream accepted for {}", conn.remote_addr());
    let mut buf = BytesMut::with_capacity(2048);

    let mut len = recv_stream_read(&mut rx, &mut buf).await?;
    let connect = loop {
        match v4::read(&mut buf, 1024 * 1024) {
            // TODO: check duplicate id
            Ok(v4::Packet::Connect(connect)) => break connect,
            Ok(_) => continue,
            Err(mqttbytes::Error::InsufficientBytes(_)) => {
                len += recv_stream_read(&mut rx, &mut buf).await?;
                debug!("recv CONNECT loop len = {}", len);
                continue;
            }
            Err(e) => return Err(Error::MQTT(e)),
        }
    };

    let client_id = connect.client_id;
    let _old_con = {
        let mut conns_list_lock = conns_list.lock().unwrap();
        !conns_list_lock.insert(client_id.clone())
    };

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
        let (tx, rx) = conn.accept().await?;
        let mapper = mapper.clone();
        let config = config.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_new_stream(tx, rx, mapper, config, remote_addr).await {
                error!("{}", e);
            }
        });
    }
}

async fn handle_new_stream(
    tx: quinn::SendStream,
    mut rx: quinn::RecvStream,
    mapper: Mapper,
    config: Arc<Config>,
    remote_addr: SocketAddr,
) -> Result<(), Error> {
    let mut buf = BytesMut::with_capacity(2048);

    recv_stream_read(&mut rx, &mut buf).await?;
    loop {
        match v4::read(&mut buf, 1024 * 1024) {
            Ok(v4::Packet::Publish(v4::Publish { topic, .. })) => {
                // ignoring first publish's payload as there are no subscribers
                // TODO: handle case when subsribing to topic that is not in mapper
                let (data_tx, db) = {
                    let mut map_lock = mapper.lock().unwrap();
                    match map_lock.get(&topic) {
                        Some(RoutingConfig { tx, db, .. }) => (tx.clone(), db.clone()),
                        None => {
                            let mut path = config.logs.path.clone();
                            path.push(&topic);
                            let db = sled::open(path)?;
                            // notification channels only need a single slot to notify
                            let (tx, rx) = flume::bounded(1);
                            map_lock.insert(
                                topic,
                                RoutingConfig {
                                    rx,
                                    tx: tx.clone(),
                                    db: db.clone(),
                                },
                            );
                            (tx, db)
                        }
                    }
                };
                debug!("new PUBLISH stream addr = {} id = {}", remote_addr, rx.id());
                return handle_publish(rx, data_tx, db, buf, remote_addr).await;
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
                let topic = &filter.path;
                let (notif_rx, db) = {
                    let mut map_lock = mapper.lock().unwrap();
                    match map_lock.get(topic) {
                        Some(RoutingConfig { rx, db, .. }) => (rx.clone(), db.clone()),
                        None => {
                            let mut path = config.logs.path.clone();
                            path.push(&topic);
                            let db = sled::open(path)?;
                            // notification channels only need a single slot to notify
                            let (tx, rx) = flume::bounded(1);
                            map_lock.insert(
                                topic.clone(),
                                RoutingConfig {
                                    rx: rx.clone(),
                                    tx,
                                    db: db.clone(),
                                },
                            );
                            (rx, db)
                        }
                    }
                };
                debug!(
                    "new SUBSCRIBE stream addr = {} id = {}",
                    remote_addr,
                    rx.id()
                );
                return handle_subscribe(tx, rx, notif_rx, db, remote_addr).await;
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
    mut stream_rx: quinn::RecvStream,
    notif_tx: NotifTx,
    db: sled::Db,
    mut buf: BytesMut,
    remote_addr: SocketAddr,
) -> Result<(), Error> {
    buf.reserve(2048);

    debug!(
        "{} :: {} [PS] publish read loop launched, cap = {}",
        remote_addr,
        stream_rx.id(),
        buf.capacity()
    );

    let mut next_key = match db.last()? {
        // TODO:
        //   use slice::array_chunks once it stabilizes.
        //   tracking issue: https://github.com/rust-lang/rust/issues/74985
        Some((k, _)) => u64::from_be_bytes([k[0], k[1], k[2], k[3], k[4], k[5], k[6], k[7]]) + 1,
        None => 0,
    };

    'outer: loop {
        let len = recv_stream_read(&mut stream_rx, &mut buf).await?;
        debug!(
            "{} :: {} [PS] read {} bytes",
            remote_addr,
            stream_rx.id(),
            len
        );

        let mut batch = sled::Batch::default();

        loop {
            let publish = match MQTTRead::read(&mut buf) {
                Ok(MQTTRead::Publish(bytes)) => bytes,
                Ok(MQTTRead::Disconnect) => break 'outer,
                Err(mqttbytes::Error::InsufficientBytes(_)) => break,
                Err(e) => return Err(Error::MQTT(e)),
            };

            batch.insert(&next_key.to_be_bytes(), &publish[..]);
            next_key += 1;
        }

        // TODO:
        //   expose config API to provide db flush size, where size = number of publish messages
        db.apply_batch(batch)?;

        if matches!(
            notif_tx.try_send(()),
            Err(flume::TrySendError::Disconnected(_))
        ) {
            return Err(Error::PubNotifTx(flume::TrySendError::Disconnected(())));
        }

        buf.reserve(2048);
    }

    Ok(())
}

async fn handle_subscribe(
    mut tx: quinn::SendStream,
    mut rx: quinn::RecvStream,
    notif_rx: NotifRx,
    db: sled::Db,
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

    let mut last_read_key = loop {
        match db.last()? {
            Some((k, _)) => {
                break u64::from_be_bytes([k[0], k[1], k[2], k[3], k[4], k[5], k[6], k[7]])
            }
            None => {
                notif_rx.recv_async().await?;
                continue;
            }
        }
    };

    for item_res in db.range::<[u8; 8], _>(last_read_key.to_be_bytes()..) {
        let (_, data) = item_res?;
        tx.write_all(&data).await?;
        last_read_key += 1;
    }
    tx.flush().await?;

    loop {
        tokio::select! {
            notif_res = notif_rx.recv_async() => {
                notif_res?;

                let mut len = 0;
                for item_res in db.range::<[u8; 8], _>(&last_read_key.to_be_bytes()..) {
                    let (_, data) = item_res?;
                    tx.write_all(&data).await?;
                    len += data.len();
                    last_read_key += 1;
                }
                tx.flush().await?;

                debug!("{} :: {} [SS] recved pub len = {}", remote_addr, rx.id(), len);
            }

            read = recv_stream_read(&mut rx, &mut recv_buf) => {
                let len = read?;
                debug!("{} :: {} [SS] read {} bytes", remote_addr, rx.id(), len);
                match v4::read(&mut recv_buf, 1024 * 1024) {
                    Ok(v4::Packet::Unsubscribe(_) | v4::Packet::Disconnect) => break,
                    Ok(_) | Err(mqttbytes::Error::InsufficientBytes(_)) => continue,
                    Err(e) => return Err(Error::MQTT(e)),
                };
            }
        }
    }

    Ok(())
}
