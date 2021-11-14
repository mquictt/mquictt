use std::net::SocketAddr;

use bytes::Bytes;
use tokio::task::JoinHandle;

#[tokio::main]
async fn main() -> Result<(), mquictt::Error> {
    pretty_env_logger::init();

    let connect_addr = ([127, 0, 0, 1], 1883).into();
    let payload = Bytes::from_static(b"hello!");
    let mut v = Vec::new();

    let mut client1 = launch_client(1, 2001, &connect_addr, payload.clone()).await?;
    let mut client2 = launch_client(2, 2002, &connect_addr, payload.clone()).await?;
    let mut client3 = launch_client(3, 2003, &connect_addr, payload.clone()).await?;

    v.extend(spawn_subs(1, &mut client1, vec![2, 3]).await?);
    v.extend(spawn_subs(2, &mut client2, vec![1, 3]).await?);
    v.extend(spawn_subs(3, &mut client3, vec![1, 2]).await?);

    for handle in v {
        handle.await.unwrap()?;
    }

    Ok(())
}

async fn launch_client(
    id: u16,
    bind_port: u16,
    connect_addr: &SocketAddr,
    payload: Bytes,
) -> Result<mquictt::Client, mquictt::Error> {
    println!("launcing clinet {}", id);

    let mut client = mquictt::Client::connect(
        &([127, 0, 0, 1], bind_port).into(),
        connect_addr,
        "localhost",
        &format!("{}", id),
    )
    .await?;

    println!("create pub {}", id);
    let mut publisher = client
        .publisher(format!("hello/{}/world", id), payload.clone())
        .await?;

    println!("launcing pub : {}", id);
    tokio::spawn(async move {
        for _ in 0..10 {
            publisher.publish(payload.clone()).await?;
        }
        let res = publisher.flush().await;
        println!("pub done {} : {}", id, res.is_ok());
        res
    });

    Ok(client)
}

async fn spawn_subs(
    id: u16,
    client: &mut mquictt::Client,
    subs: Vec<u16>,
) -> Result<Vec<JoinHandle<Result<(), mquictt::Error>>>, mquictt::Error> {
    let mut v = Vec::with_capacity(subs.len());
    for sub in subs {
        let mut subsriber = client.subsriber(format!("hello/{}/world", sub)).await?;
        println!("launcing sub : {} {}", id, sub);
        let handle = tokio::spawn(async move {
            for _ in 0..10 {
                let _payload = subsriber.read().await?;
            }
            println!("sub done {} : {}", id, sub);
            Ok(())
        });
        v.push(handle);

    }
    Ok(v)
}
