use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, RwLock},
};

use bytes::{Bytes, BytesMut};
use mqttbytes::v4;

use crate::{Connection, Error, QuicServer};

type SubReqTx = flume::Sender<flume::Sender<Bytes>>;
type Mapper = Arc<RwLock<HashMap<String, SubReqTx>>>;

pub async fn server(addr: &SocketAddr) -> Result<(), Error> {
    let mut listener = QuicServer::new(addr)?;
    let mapper: Mapper = Arc::new(RwLock::new(HashMap::default()));

    loop {
        let conn = match listener.accept().await {
            Ok(conn) => conn,
            Err(Error::ConnectionBroken) => return Ok(()),
            e => e?,
        };
        let mapper = mapper.clone();

        tokio::spawn(connection_handler(conn, mapper));
    }
}

async fn connection_handler(mut conn: Connection, mapper: Mapper) -> Result<(), Error> {
    let (mut tx, mut rx) = conn.accept().await?;
    let mut buf = BytesMut::new();

    rx.read(&mut buf).await?;
    loop {
        match v4::read(&mut buf, 1024 * 1024) {
            Ok(v4::Packet::Connect(_)) => break,
            Ok(_) => continue,
            Err(mqttbytes::Error::InsufficientBytes(_)) => {
                rx.read(&mut buf).await?;
                continue;
            }
            Err(e) => return Err(Error::MQTT(e)),
        }
    }

    buf.clear();
    if let Err(e) = v4::ConnAck::new(v4::ConnectReturnCode::Success, false).write(&mut buf) {
        return Err(Error::MQTT(e));
    }
    let _write = tx.write(&buf).await?;

    let (tx, rx): (SubReqTx, _) = flume::bounded(1024);

    buf.clear();
    loop {
        // for each publish topic, spawn a new thread and add the corresponding entry in `mapper`
        //
        // for each subsription, send a request for the topic using sender from `mapper`, and the
        // sender handle sent over is same for all subsriptions
        unimplemented!()
    }

    Ok(())
}
