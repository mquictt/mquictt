use bytes::Bytes;
use log::error;

#[tokio::main]
async fn main() -> Result<(), mquictt_client::Error> {
    pretty_env_logger::init();

    // create a client
    let mut client = mquictt_client::Client::connect(
        &([127, 0, 0, 1], 2000).into(),
        &([127, 0, 0, 1], 1883).into(),
        "localhost",
        "0",
        mquictt_client::Config::read(&"mquictt-client/examples/client.json").unwrap(),
    )
    .await?;

    // create a publisher for a particular topic
    let mut publisher = client
        .publisher("hello/world", Bytes::from("hello"))
        .await?;

    let mut subscriber = client.subscriber("hello/world").await?;
    publisher.publish(Bytes::from("hello again!")).await?;
    publisher.flush().await.unwrap();
    tokio::spawn(async move {
        for i in 0..100 {
            if let Err(e) = publisher.publish(Bytes::from(format!("{}!", i))).await {
                error!("{}", e);
            }
            if let Err(e) = publisher.flush().await {
                error!("{}", e);
            }
        }
        if let Err(e) =publisher.close().await {
            error!("{}", e);
        }
    });

    // Read from subscriber
    for _ in 0..101 {
        println!(
            "{}",
            std::str::from_utf8(&subscriber.read().await?).unwrap()
        );
    }

    subscriber.close().await?;
    client.close().await?;

    Ok(())
}
