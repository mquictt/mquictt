use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, RwLock},
};

use bytes::BytesMut;
use futures::{stream::FuturesUnordered, StreamExt};
use log::*;
use mqttbytes::v4;
use slab::Slab;

use crate::{recv_stream_read, Config, Connection, Error, QuicServer};

type DataTx = flume::Sender<v4::Publish>;
type DataRx = flume::Receiver<v4::Publish>;

type SubReqTx = flume::Sender<DataTx>;
type SubReqRx = flume::Receiver<DataTx>;
type Mapper = Arc<RwLock<HashMap<String, SubReqTx>>>;

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
    let (mut tx, mut rx) = conn.accept().await?;
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
    loop {
        let (tx, rx) = conn.accept().await?;
        tokio::spawn(handle_new_stream(
            tx,
            rx,
            mapper.clone(),
            conn.remote_addr(),
        ));

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
    mapper: Mapper,
    remote_addr: SocketAddr,
) -> Result<(), Error> {
    let mut buf = BytesMut::with_capacity(2048);

    recv_stream_read(&mut rx, &mut buf).await?;
    loop {
        match v4::read(&mut buf, 1024 * 1024) {
            Ok(v4::Packet::Publish(v4::Publish { topic, .. })) => {
                // ignoring first publish's payload as there are no subscribers
                // TODO: handle case when subsribing to topic that is not in mapper
                let (sub_req_tx, sub_req_rx) = flume::bounded(1024);
                {
                    let mut map_writer = mapper.write().unwrap();
                    map_writer.insert(topic, sub_req_tx);
                }
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
                    // TODO: handle case when subsribing to topic that is not in mapper
                    let sub_req_tx = map_reader.get(&filter.path).unwrap();
                    // waiting blockingly as we are not allowed to await when holding a lock
                    sub_req_tx.send(data_tx)?;
                }
                debug!(
                    "new SUBSCRIBE stream addr = {} id = {}",
                    remote_addr,
                    rx.id()
                );
                return handle_subscribe(tx, rx, data_rx, remote_addr).await;
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

    loop {
        tokio::select! {
            read = recv_stream_read(&mut rx, &mut buf) => {
                let len = read?;
                debug!("{} :: {} [PS] read {} bytes", remote_addr, rx.id(), len);

                let publish = match v4::read(&mut buf, 1024 * 1024) {
                    Ok(v4::Packet::Publish(publish)) => publish,
                    Ok(_) | Err(mqttbytes::Error::InsufficientBytes(_)) => continue,
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
                }
            }

            sub_req = sub_req_rx.recv_async() => {
                // if cannot be recved then mapper has been droped, exit normally
                let data_tx = match sub_req {
                    Ok(v) => v,
                    Err(_) => break,
                };
                subscribers.insert(Arc::new(data_tx));
            }

            send_opt = send_queue.next(), if !send_queue_empty => {
                match send_opt {
                    Some(Some((slab_id, _e))) => {
                        subscribers.remove(slab_id);
                    },
                    Some(None) => {}
                    None => send_queue_empty = false,
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
    let mut buf = BytesMut::new();
    let suback = v4::SubAck::new(
        0,
        vec![v4::SubscribeReasonCode::Success(mqttbytes::QoS::AtMostOnce)],
    );

    loop {
        tokio::select! {
            data_res = data_rx.recv_async() => {
                let data = data_res?;
                // TODO: use try_recv in loop for buffering
                if let Err(e) = data.write(&mut buf) {
                    return Err(Error::MQTT(e));
                };
                tx.write_all(&buf).await?;
            }

            read = recv_stream_read(&mut rx, &mut buf) => {
                let len = read?;
                debug!("{} :: {} [SS] read {} bytes", remote_addr, rx.id(), len);
                match v4::read(&mut buf, 1024 * 1024) {
                    Ok(v4::Packet::Unsubscribe(_)) => {
                        if let Err(e) = suback.write(&mut buf) {
                            return Err(Error::MQTT(e));
                        };
                        tx.write_all(&buf).await?;
                        break
                    },
                    Ok(_) | Err(mqttbytes::Error::InsufficientBytes(_)) => continue,
                    Err(e) => return Err(Error::MQTT(e)),
                };
            }
        }
    }

    Ok(())
}
