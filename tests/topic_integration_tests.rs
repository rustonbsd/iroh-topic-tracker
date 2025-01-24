use anyhow::Result;
use iroh::Endpoint;
use iroh_topic_tracker::topic_tracker::{Topic, TopicTracker};
use rand::{rngs::OsRng, Rng};
use std::sync::Arc;

#[tokio::test]
async fn test_topic_lifecycle() -> Result<()> {
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
