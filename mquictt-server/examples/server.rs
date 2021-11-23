#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    mquictt_server::server(
        &([127, 0, 0, 1], 1883).into(),
        mquictt_server::Config::read(&"mquictt-server/examples/server.json").unwrap(),
    )
    .await
    .unwrap();
}
