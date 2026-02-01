use std::time::Duration;

use dht::SigningKey;
use futures_lite::StreamExt;
use iroh::{Endpoint, SecretKey, protocol::Router};
use iroh_gossip::api::Event;
use iroh_gossip::net::Gossip;
use tokio::time::timeout;

use iroh_topic_tracker::{TopicDiscoveryConfig, TopicDiscoveryExt, TopicDiscoveryHook};

#[tokio::test(flavor = "multi_thread")]
async fn topic_tracker_gossip_integration_test() -> anyhow::Result<()> {
    // Create two endpoints with different node IDs
    let secret_key0 = SecretKey::generate(&mut rand::rng());
    let signing_key0 = SigningKey::from_bytes(&secret_key0.to_bytes());
    let hook_0 = TopicDiscoveryHook::new();

    let secret_key1 = SecretKey::generate(&mut rand::rng());
    let signing_key1 = SigningKey::from_bytes(&secret_key1.to_bytes());
    let hook_1 = TopicDiscoveryHook::new();

    let endpoint0 = Endpoint::builder()
        .secret_key(secret_key0)
        .hooks(hook_0.clone())
        .bind()
        .await?;

    let endpoint1 = Endpoint::builder()
        .secret_key(secret_key1)
        .hooks(hook_1.clone())
        .bind()
        .await?;

    let gossip0 = Gossip::builder().spawn(endpoint0.clone());
    let gossip1 = Gossip::builder().spawn(endpoint1.clone());

    let _router0 = Router::builder(endpoint0.clone())
        .accept(iroh_gossip::ALPN, gossip0.clone())
        .spawn();

    let _router1 = Router::builder(endpoint1.clone())
        .accept(iroh_gossip::ALPN, gossip1.clone())
        .spawn();

    // Test topic - using a unique topic for this test
    let topic_id = format!("test_topic_{}", rand::random::<u32>()).into_bytes();

    let config0 = TopicDiscoveryConfig::new(signing_key0, hook_0)
        .max_peers_per_round(Some(5));

    let config1 = TopicDiscoveryConfig::new(signing_key1, hook_1)
        .max_peers_per_round(Some(5));

    // Subscribe both nodes to the same topic
    let (sender0, mut receiver0, handle0) = gossip0
        .subscribe_with_discovery(topic_id.clone(), vec![], config0)
        .await?;

    let (sender1, mut receiver1, handle1) = gossip1
        .subscribe_with_discovery(topic_id, vec![], config1)
        .await?;

    // Wait for node 0 to see a neighbor
    timeout(Duration::from_secs(15), async {
        loop {
            match receiver0.next().await {
                Some(Ok(Event::NeighborUp(_))) => break,
                Some(Err(e)) => panic!("Error receiving event: {}", e),
                None => panic!("Stream ended"),
                _ => {}
            }
        }
    }).await.expect("Node 0 failed to find neighbor");

    // Wait for node 1 to see a neighbor
    timeout(Duration::from_secs(15), async {
        loop {
            match receiver1.next().await {
                Some(Ok(Event::NeighborUp(_))) => break,
                Some(Err(e)) => panic!("Error receiving event: {}", e),
                None => panic!("Stream ended"),
                _ => {}
            }
        }
    }).await.expect("Node 1 failed to find neighbor");

    // Send a message from node 0
    let test_message = format!("test message from node0: {}", rand::random::<u32>());
    sender0.broadcast(test_message.clone().into()).await?;

    // Wait for node 1 to receive the message with a timeout
    let received = timeout(Duration::from_secs(30), async {
        while let Some(event) = receiver1.next().await {
            if let Ok(Event::Received(msg)) = event {
                let content = String::from_utf8(msg.content.to_vec())?;
                if content == test_message {
                    return Ok::<bool, anyhow::Error>(true);
                }
            }
        }
        Ok(false)
    })
    .await??;

    println!("Node 1 received message: {}", received);

    assert!(received, "Node 1 should receive message from Node 0");

    // Clean up
    handle0.stop();
    handle1.stop();

    // Also test sending from node 1 to node 0
    let test_message2 = format!("test message from node1: {}", rand::random::<u32>());
    sender1.broadcast(test_message2.clone().into()).await?;

    let received2 = timeout(Duration::from_secs(10), async {
        while let Some(event) = receiver0.next().await {
            if let Ok(Event::Received(msg)) = event {
                let content = String::from_utf8(msg.content.to_vec())?;
                if content == test_message2 {
                    return Ok::<bool, anyhow::Error>(true);
                }
            }
        }
        Ok(false)
    })
    .await??;

    assert!(received2, "Node 0 should receive message from Node 1");

    Ok(())
}