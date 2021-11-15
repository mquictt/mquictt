# MQuicTT

<img align="right" src="https://github.com/mquictt/mquictt/raw/main/docs/logo.png" height="150px" alt="MQuicTT logo">

<br>

**🚧 This is a pre-alpha project, tread carefully 🚧**

A rustlang utility/library for [MQTT] over [QUIC].

QUIC allows us to send data over multiple concurrent streams. We can leverage this ability to multiplex the pub-sub architecture of MQTT. This means that:
- when a MQTT client is subscribed to multiple topics, it can get data for these subscriptions concurrently. Even if data for some subscriptions gets delayed, rest of the subscriptions can carry on. When running MQTT over TCP, a single subscription's data being delayed can block rest of the data as well (see [head-of-line blocking]). Similarly on the server side, the server can send data for multiple subscriptions concurrently.
- when a MQTT client publishes messages on multiple topics, these can again me sent concurrently, and the server can receive them concurrently.

QUIC also provides at-most-once message delivery guarantees, and thus the MQTT running on top of it gets QoS2 support by default. Thus the library does not give an option to configure the QoS.

As QUIC is basically a state-machine slapped on top of UDP, the only change needed is at application level. We can use the UDP APIs exposed by the Rust's standard library and we are good to go!.

## Usage

To create a MQTT server:
```rust
async fn spawn_server() {
    mquictt::server(
        &([127, 0, 0, 1], 1883).into(),
        mquictt::Config::read(&"server.json").unwrap(),
    )
    .await
    .unwrap();
}
```

To create a MQTT client:
```rust
use bytes::Bytes;

async fn spawn_client() {
    // create a client
    let mut client = mquictt::Client::connect(
        &([127, 0, 0, 1], 2000).into(),
        &([127, 0, 0, 1], 1883).into(),
        "localhost",
        "0",
        mquictt::Config::read(&"client.json").unwrap(),
    )
    .await.unwrap();

    // create a publisher for a particular topic
    let mut publisher = client
        .publisher("hello/world", Bytes::from("hello"))
        .await.unwrap();
    publisher.publish(Bytes::from("hello again!")).await.unwrap();

    // create a subscriber
    let mut subscriber = client.subscriber("hello/world").await.unwrap();
    let read = subscriber.read().await.unwrap();
    println!("{}", std::str::from_utf8(&read).unwrap());

    Ok(())
}
```

## Acknowledgements
We use:
- [quinn-rs/quinn][quinn] underneath to interface with QUIC
- [bytebeamio/mqttbytes][mqttbytes] to parse MQTT packets

[MQTT]: https://mqtt.org/
[QUIC]: https://en.wikipedia.org/wiki/QUIC
[quinn]: https://github.com/quinn-rs/quinn
[mqttbytes]: https://github.com/bytebeamio/rumqtt/tree/master/mqttbytes
[head-of-line blocking]: https://en.wikipedia.org/wiki/Head-of-line_blocking
