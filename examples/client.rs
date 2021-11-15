use bytes::Bytes;

#[tokio::main]
async fn main() -> Result<(), mquictt::Error> {
    pretty_env_logger::init();

    // create a client
    let mut client = mquictt::Client::connect(
        &([127, 0, 0, 1], 2000).into(),
        &([127, 0, 0, 1], 1883).into(),
        "localhost",
        "0",
        mquictt::Config::read(&"examples/client.json").unwrap(),
    )
    .await?;

    dbg!();

    // create a publisher for a particular topic
    let mut publisher = client
        .publisher("hello/world", Bytes::from("hello"))
        .await?;
    publisher.publish(Bytes::from("hello again!")).await?;

    // create a subscriber
    let mut subscriber = client.subscriber("hello/world").await?;
    println!(
        "{}",
        std::str::from_utf8(&subscriber.read().await?).unwrap()
    );

    Ok(())
}
