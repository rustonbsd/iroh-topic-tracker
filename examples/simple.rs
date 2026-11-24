use iroh::{Endpoint, SecretKey, protocol::Router};
use iroh_gossip::Gossip;
use std::time::Duration;

use iroh_topic_tracker::{TopicDiscoveryConfig, TopicDiscoveryExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let secret_key = SecretKey::generate();

    let endpoint = Endpoint::builder(iroh::endpoint::presets::N0)
        .secret_key(secret_key.clone())
        .bind()
        .await?;

    let gossip = Gossip::builder().spawn(endpoint.clone());

    let _router = Router::builder(endpoint.clone())
        .accept(iroh_gossip::ALPN, gossip.clone())
        .spawn();

    let topic_id = "testnet".as_bytes().to_vec();
    let config = TopicDiscoveryConfig::builder(endpoint)
        .max_peers_per_round(Some(5))
        .connection_timeout(Duration::from_secs(10))
        .build();

    println!("Starting subscription to topic...");
    let (sender, _receiver, discovery_handle) = gossip
        .subscribe_with_discovery_joined(topic_id, vec![], config)
        .await?;

    println!("Subscribed to topic and joined the network.");

    println!("Broadcasting hello world...");
    sender
        .broadcast(format!("hello world {}", rand::random::<u32>()).into())
        .await?;

    discovery_handle.stop();

    Ok(())
}
