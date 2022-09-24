use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, RwLock},
};

use bytes::BytesMut;
use flume::Sender;
use futures::stream::{FuturesUnordered, StreamExt};
use log::*;
use mqttbytes::v4;
use slab::Slab;

use mquictt_core::{recv_stream_read, Connection, Publish, QuicServer};
pub use mquictt_core::{Config, Error};

type DataTx = flume::Sender<Publish>;
type DataRx = flume::Receiver<Publish>;

type SubReqTx = flume::Sender<DataTx>;
type SubReqRx = flume::Receiver<DataTx>;
type Mapper = Arc<RwLock<HashMap<String, (Option<SubReqRx>, SubReqTx)>>>;

/// Spawns a new server that listens for incoming MQTT connects from clients at the given address.
///
/// ```
/// tokio::spawn(server([127, 0, 0, 1], 1883).into(), mquictt::Config::empty());
/// ```
pub async fn server(addr: &SocketAddr, config: Arc<Config>) -> Result<(), Error> {
    let mut listener = QuicServer::new(config, addr)?;
    info!("QUIC server launched at {}", listener.local_addr());
    let mapper: Mapper = Arc::new(RwLock::new(HashMap::default()));

    loop {
        let conn = match listener.accept().await {
            Ok(conn) => conn,
            Err(Error::ConnectionBroken) => {
                log::warn!("conn broker {}", listener.local_addr());
                return Ok(());
            }
            e => e?,
        };
        let mapper = mapper.clone();

        info!("accepted conn from {}", conn.remote_addr());
        tokio::spawn(connection_handler(conn, mapper));
    }
}

async fn connection_handler(mut conn: Connection, mapper: Mapper) -> Result<(), Error> {
    debug!("connection handler spawned for {}", conn.remote_addr());
    let (mut tx, mut rx) = conn.accept_stream().await?;
    debug!("stream accepted for {}", conn.remote_addr());
    let mut buf = BytesMut::with_capacity(2048);

    let mut len = recv_stream_read(&mut rx, &mut buf).await?;
    loop {
        match v4::read(&mut buf, 1024 * 1024) {
            // TODO: check duplicate id
            Ok(v4::Packet::Connect(_)) => break,
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
    let (close_tx, close_rx) = flume::bounded(1);
    loop {
        tokio::select! {
            streams_result = conn.accept_stream() => {
                let (tx, rx) = streams_result?;
                let mapper = mapper.clone();
                let close_tx = close_tx.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_new_stream(tx, rx, mapper, remote_addr, close_tx).await {
                        error!("{}", e);
                    }
                });
            }

            _ = close_rx.recv_async() => {
                info!("Connection closed: {}", conn.remote_addr());
                return Ok(())
            }
        }
    }
}

async fn handle_new_stream(
    tx: quinn::SendStream,
    mut rx: quinn::RecvStream,
    mapper: Mapper,
    remote_addr: SocketAddr,
    close_tx: Sender<()>,
) -> Result<(), Error> {
    let mut buf = BytesMut::with_capacity(2048);

    recv_stream_read(&mut rx, &mut buf).await?;
    loop {
        match v4::read(&mut buf, 1024 * 1024) {
            Ok(v4::Packet::Publish(v4::Publish { topic, .. })) => {
                // ignoring first publish's payload as there are no subscribers
                let sub_req_rx = {
                    let mut map_writer = mapper.write().unwrap();
                    match map_writer.get_mut(&topic) {
                        Some((sub_req_rx, _)) => sub_req_rx.take().unwrap(),
                        None => {
                            let (sub_req_tx, sub_req_rx) = flume::bounded(1024);
                            map_writer.insert(topic, (None, sub_req_tx));
                            sub_req_rx
                        }
                    }
                };

                debug!("new PUBLISH stream addr = {} id = {}", remote_addr, rx.id());
                return handle_publish(rx, sub_req_rx, buf, remote_addr).await;
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
                let (data_tx, data_rx) = flume::bounded(1024);
                {
                    let map_reader = mapper.read().unwrap();
                    match map_reader.get(&filter.path) {
                        Some((_, sub_req_tx)) => sub_req_tx.send(data_tx)?,
                        None => {
                            drop(map_reader);
                            let (sub_req_tx, sub_req_rx) = flume::bounded(1024);
                            let mut map_writer = mapper.write().unwrap();
                            map_writer
                                .insert(filter.path.to_owned(), (Some(sub_req_rx), sub_req_tx));
                        }
                    }
                    // waiting blockingly as we are not allowed to await when holding a lock
                }
                debug!(
                    "new SUBSCRIBE stream addr = {} id = {}",
                    remote_addr,
                    rx.id()
                );
                return handle_subscribe(tx, rx, data_rx, remote_addr).await;
            }
            Ok(v4::Packet::Disconnect) => {
                info!("Disconnecting from: {}", remote_addr);
                if let Err(e) = close_tx.send(()) {
                    error!("Couldn't use channel to close connection: {}", e)
                }
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
    sub_req_rx: SubReqRx,
    mut buf: BytesMut,
    remote_addr: SocketAddr,
) -> Result<(), Error> {
    let mut subscribers: Slab<Arc<DataTx>> = Slab::with_capacity(1024);
    let mut send_queue = FuturesUnordered::new();
    let mut send_queue_empty = true;
    buf.reserve(2048);

    debug!(
        "{} :: {} [PS] publish read loop launched, cap = {}",
        remote_addr,
        rx.id(),
        buf.capacity()
    );

    'outer: loop {
        tokio::select! {
            read = recv_stream_read(&mut rx, &mut buf) => {
                let len = read?;
                debug!("{} :: {} [PS] read {} bytes", remote_addr, rx.id(), len);

                loop {
                    let publish = match Publish::read(&mut buf) {
                        Ok(publish) => publish,
                        Err(mqttbytes::Error::InsufficientBytes(_)) => continue 'outer,
                        Err(e) => return Err(Error::MQTT(e)),
                    };

                    for (slab_id, subsriber) in subscribers.iter() {
                        let subsriber = subsriber.clone();
                        let publish = publish.clone();
                        send_queue.push(async move {
                            match subsriber.send_async(publish).await {
                                Ok(_) => None,
                                Err(e) => Some((slab_id, e)),
                            }
                        });
                        send_queue_empty = false;
                    }

                    buf.reserve(2048);
                }

            }

            sub_req = sub_req_rx.recv_async() => {
                // if cannot be recved then mapper has been droped, exit normally
                let data_tx = match sub_req {
                    Ok(v) => v,
                    Err(_) => break,
                };
                subscribers.insert(Arc::new(data_tx));
                debug!("{} :: {} [PS] recved sub req, total = {}", remote_addr, rx.id(), subscribers.len());
            }

            send_opt = send_queue.next(), if !send_queue_empty => {
                match send_opt {
                    Some(Some((slab_id, e))) => {
                        subscribers.remove(slab_id);
                        error!("{} :: {} [PS] 1 sub removed, total = {} reason = {}", remote_addr, rx.id(), subscribers.len(), e);
                    },
                    Some(None) => debug!("sent some data // remove this later"),
                    None => send_queue_empty = true,
                }
            }
        }
    }

    Ok(())
}

async fn handle_subscribe(
    mut tx: quinn::SendStream,
    mut rx: quinn::RecvStream,
    data_rx: DataRx,
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
            data_res = data_rx.recv_async() => {
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
