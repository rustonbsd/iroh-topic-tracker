use anyhow::Result;
use futures_lite::StreamExt;
use iroh::{Endpoint, SecretKey};
use iroh_gossip::net::{Event, Gossip, GossipEvent};
use iroh_topic_tracker::topic_tracker::Topic;

// Import optional feature to work with iroh-gossip
// new functions    spawn_with_auto_discovery
// and              subscribe_and_join_with_auto_discovery
use iroh_topic_tracker::integrations::AutoDiscoveryBuilder;
use iroh_topic_tracker::integrations::AutoDiscoveryGossip;

#[cfg(feature="iroh-gossip-auto-discovery")]
#[tokio::main]
async fn main() -> Result<()> {
    let secret_key = SecretKey::generate(rand::rngs::OsRng);

    let endpoint = Endpoint::builder()
        .secret_key(secret_key)
        .discovery_n0()
        .discovery_dht()
        .bind()
        .await?;

    let gossip = Gossip::builder()
        .spawn_with_auto_discovery(endpoint.clone())
        .await?;
    let _router = iroh::protocol::Router::builder(endpoint.clone())
        .accept(iroh_gossip::ALPN, gossip.gossip.clone())
        .spawn()
        .await?;

    let topic = Topic::from_passphrase("my-iroh-gossip-topic");

    let (sink, mut stream) = gossip.subscribe_and_join(topic.into()).await?.split();

    tokio::spawn(async move {
        while let Some(event) = stream.next().await {
            if let Ok(Event::Gossip(GossipEvent::Received(msg))) = event {
                println!(
                    "Message from {}: {}",
                    &msg.delivered_from.to_string()[0..8],
                    String::from_utf8(msg.content.to_vec()).unwrap()
                );
            } else if let Ok(Event::Gossip(GossipEvent::Joined(peers))) = event{
                for peer in peers {
                    println!("Joined by {}",&peer.to_string()[0..8]);
                }
            }
        }
    });

    let mut buffer = String::new();
    let stdin = std::io::stdin();
    loop {
        print!("> ");
        stdin.read_line(&mut buffer).unwrap();
        sink.broadcast(buffer.clone().replace("\n","").into()).await.unwrap();
        buffer.clear();
    }
}
