use std::time::Duration;

use dht::SigningKey;
use futures_lite::StreamExt;
use iroh::{Endpoint, SecretKey, protocol::Router};
use iroh_gossip::net::Gossip;

use iroh_topic_tracker::{TopicDiscoveryConfig, TopicDiscoveryExt};
use tokio::time::sleep;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let secret_key = SecretKey::generate(&mut rand::rng());
    let signing_key = SigningKey::from_bytes(&secret_key.to_bytes());

    let endpoint = Endpoint::builder()
        .secret_key(secret_key.clone())
        .bind()
        .await?;

    let gossip = Gossip::builder().spawn(endpoint.clone());

    let _router = Router::builder(endpoint)
        .accept(iroh_gossip::ALPN, gossip.clone())
        .spawn();

    let topic_id = "my_topi2c".as_bytes().to_vec();
    let config = TopicDiscoveryConfig::new(signing_key).discovery_interval(Duration::from_secs(30));

    let (sender, mut receiver, discovery_handle) = gossip
        .subscribe_with_discovery_joined(topic_id, vec![], config)
        .await?;

    println!("Subscribed to topic and joined the network.");

    sleep(Duration::from_secs(2)).await;
    sender.broadcast(b"hello world".to_vec().into()).await?;

    println!("send helo world");
    while let Some(event) = receiver.next().await {
        match event? {
            iroh_gossip::api::Event::Received(msg) => {
                println!("Got: {:?}", msg.content);
            }
            iroh_gossip::api::Event::NeighborUp(peer) => {
                println!("Peer joined: {}", peer.fmt_short());
            }
            _ => {}
        }
    }

    discovery_handle.stop();

    Ok(())
}
