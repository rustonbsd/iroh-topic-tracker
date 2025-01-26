use std::{sync::Arc, time::Duration};

use iroh::{Endpoint, SecretKey};
use iroh_topic_tracker::topic_tracker::{Topic, TopicTracker};
use rand::{rngs::OsRng, Rng};
use tokio::time::sleep;


#[tokio::test]
async fn topic_tracker_gossip_integration_test() -> anyhow::Result<()> {
    // Test topic creation and conversion
    let topic = Topic::new(OsRng.gen::<[u8; 32]>());
    
    // Create endpoint0 with nodeid0 and topic_tracker0
    let endpoint0 = Endpoint::builder()
        .discovery_n0()
        .secret_key(SecretKey::generate(rand::rngs::OsRng))
        .bind()
        .await?;
    // Create endpoint1 with nodeid1 and topic_tracker1
    let endpoint1 = Endpoint::builder()
        .discovery_n0()
        .secret_key(SecretKey::generate(rand::rngs::OsRng))
        .bind()
        .await?;

    // These topic trackers will have different NodeId's
    let topic_tracker0 = Arc::new(TopicTracker::new(&endpoint0));
    let topic_tracker1 = Arc::new(TopicTracker::new(&endpoint1));
    
    // Test getting nodes for a topic (should be empty initially own node id not reported to requester)
    let nodes0 = topic_tracker0.clone().get_topic_nodes(&topic).await?;
    assert!(nodes0.is_empty());
    println!("Query with node_id0 expected to be empty: {nodes0:?}");
    
    sleep(Duration::from_millis(10)).await;

    let nodes1 = topic_tracker1.clone().get_topic_nodes(&topic).await?;
    println!("Query with node_id1 expected to contain node_id0: {nodes1:?}");
    assert_eq!(nodes1[0].to_string(),topic_tracker0.node_id.to_string());

    Ok(())
}


#[tokio::test]
async fn test_topic_lifecycle() -> anyhow::Result<()> {
    // Test topic creation and conversion
    let topic = Topic::new(OsRng.gen::<[u8; 32]>());
    
    // Create endpoint and topic tracker
    let endpoint = Endpoint::builder()
        .discovery_n0()
        .bind()
        .await?;
    
    let topic_tracker = Arc::new(TopicTracker::new(&endpoint));
    
    // Test getting nodes for a topic (should be empty initially)
    let nodes = topic_tracker.clone().get_topic_nodes(&topic).await?;
    assert!(nodes.is_empty());
    
    // Test topic string representation
    let topic_str = nodes[0].to_string();
    assert!(!topic_str.is_empty());
    
    // Test conversion to/from iroh_gossip::proto::TopicId
    let topic_id: iroh_gossip::proto::TopicId = topic.clone().into();
    let topic_back: Topic = topic_id.into();
    assert_eq!(topic, topic_back.into());

    Ok(())
}
