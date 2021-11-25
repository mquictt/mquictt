use bytes::Bytes;
use log::error;
use tokio::select;

#[tokio::main]
async fn main() -> Result<(), mquictt_client::Error> {
    pretty_env_logger::init();

    // create 2 clients
    let mut client = mquictt_client::Client::connect(
        &([127, 0, 0, 1], 2000).into(),
        &([127, 0, 0, 1], 1883).into(),
        "localhost",
        "0",
        mquictt_client::Config::read(&"mquictt-client/examples/client.json").unwrap(),
    )
    .await?;
    let mut client1 = mquictt_client::Client::connect(
        &([127, 0, 0, 1], 2001).into(),
        &([127, 0, 0, 1], 1883).into(),
        "localhost",
        "1",
        mquictt_client::Config::read(&"mquictt-client/examples/client1.json").unwrap(),
    )
    .await?;

    // create publishers for same topic with both clients
    let mut publisher = client
        .publisher("hello/world", Bytes::from("hello"))
        .await?;
    let mut publisher1 = client1
        .publisher("hello/world", Bytes::from("hello"))
        .await?;

    // create subscribers for same topic with both clients
    let mut subscriber = client.subscriber("hello/world").await?;
    let mut subscriber1 = client1.subscriber("hello/world").await?;

    // publish and flush from a publisher, read from both sub
    publisher.publish(Bytes::from("hello again!"))?;
    publisher.flush().await?;

    println!(
        "0: {}\n1: {}",
        std::str::from_utf8(&subscriber.read().await?).unwrap(),
        std::str::from_utf8(&subscriber1.read().await?).unwrap()
    );

    // Publish multiple times to same topic, send after each iteration
    tokio::spawn(async move {
        for i in 0..100 {
            if let Err(e) = publisher.publish(Bytes::from(format!("0: {}!", i))) {
                error!("{}", e);
            }
            if let Err(e) = publisher.flush().await {
                error!("{}", e);
            }
        }
        if let Err(e) = publisher.close().await {
            error!("{}", e);
        }
    });

    // Publish multiple times to same topic, send after all iterations
    tokio::spawn(async move {
        for i in 0..100 {
            if let Err(e) = publisher1.publish(Bytes::from(format!("1: {}!", i))) {
                error!("{}", e);
            }
        }
        if let Err(e) = publisher1.flush().await {
            error!("{}", e);
        }
        if let Err(e) = publisher1.close().await {
            error!("{}", e);
        }
    });

    // Readloop for both subscribers
    loop {
        select! {
            sub = subscriber.read() => {
                println!(
                    "0: {}",
                    std::str::from_utf8(&sub?).unwrap(),);
            }

            sub1 = subscriber1.read() => {
                println!(
                    "1: {}",
                    std::str::from_utf8(&sub1?).unwrap()
                );
            }
        }
    }
}
