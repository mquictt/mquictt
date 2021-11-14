#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    mquictt::server(
        &([127, 0, 0, 1], 1883).into(),
        mquictt::Config::read(&"examples/config.json".to_owned()).unwrap(),
    )
    .await
    .unwrap();
}
