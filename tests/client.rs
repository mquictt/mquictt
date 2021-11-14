#![feature(test)]
extern crate test;

use std::sync::Arc;

use anyhow::{Context, Error};
use bytes::Bytes;
use figment::{
    providers::{Data, Json},
    Figment,
};
use mquictt::Config;
use tokio::runtime::Runtime;

#[bench]
fn benchmark_single_topic(b: &mut test::Bencher) -> Result<(), Error> {
    let config: Config = Figment::new()
        .merge(Data::<Json>::file("./config.json"))
        .extract()
        .with_context(|| format!("Config error"))?;
    let config = Arc::new(config);

    let rt = Runtime::new()?;
    rt.block_on(async {
        let mut conn = mquictt::Client::connect(
            &"127.0.0.1:55555".parse().unwrap(),
            &"127.0.0.1:55555".parse().unwrap(),
            "localhost",
            "",
            config,
        )
        .await
        .unwrap();
        let init_payload = Bytes::from(vec![1, 3, 3, 7]);

        let mut publisher = conn.publisher("hello/world", init_payload).await.unwrap();
        let mut x = 0;
        b.iter(|| {
            x += 1;
            let payload = Bytes::from(vec![x]);
            publisher.publish(payload);
        });
    });

    Ok(())
}
