use bytes::Bytes;

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

    // create a subscriber
    println!(
        "{}",
        std::str::from_utf8(&subscriber.read().await?).unwrap()
    );

    Ok(())
}
