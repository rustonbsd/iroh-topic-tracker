
use dht::SigningKey;
use futures_lite::StreamExt;
use iroh::{Endpoint, SecretKey, protocol::Router};
use iroh_gossip::net::Gossip;

use iroh_topic_tracker::{TopicDiscoveryConfig, TopicDiscoveryExt, TopicDiscoveryHook};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("iroh_topic_tracker=debug,gossip_chat=debug,warn")),
        )
        .init();

    let secret_key = SecretKey::generate(&mut rand::rng());
    let signing_key = SigningKey::from_bytes(&secret_key.to_bytes());
    
    let hook = TopicDiscoveryHook::new();
    let endpoint = Endpoint::builder()
        .secret_key(secret_key.clone())
        .hooks(hook.clone())
        .bind()
        .await?;

    let gossip = Gossip::builder().spawn(endpoint.clone());

    let _router = Router::builder(endpoint.clone())
        .accept(iroh_gossip::ALPN, gossip.clone())
        .spawn();

    let topic_id = "testnet".as_bytes().to_vec();
    let config = TopicDiscoveryConfig::new(signing_key, hook).max_peers_per_round(Some(5));

    tracing::info!("Starting subscription to topic...");
    let (sender, mut receiver, discovery_handle) = gossip
        .subscribe_with_discovery_joined(topic_id, vec![], config)
        .await?;

    tracing::info!("Subscribed to topic and joined the network.");
    println!("Subscribed to topic and joined the network.");

    tracing::info!("Broadcasting hello world...");
    sender.broadcast(format!("hello world {}",rand::random::<u32>()).into()).await?;

    tracing::info!("Broadcast sent, waiting for events...");
    println!("send hello world");
    while let Some(event) = receiver.next().await {
        match event? {
            iroh_gossip::api::Event::Received(msg) => {
                tracing::info!("Received message: {:?}", msg.content);
                println!("Got: {:?}", msg.content);
            }
            iroh_gossip::api::Event::NeighborUp(peer) => {
                tracing::info!("Neighbor up: {}", peer.fmt_short());
                println!("Peer joined: {}", peer.fmt_short());
            }
            other => {
                tracing::debug!("Other event: {:?}", other);
            }
        }
    }

    discovery_handle.stop();

    Ok(())
}
