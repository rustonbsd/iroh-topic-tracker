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
    let nodes0 = <TopicTracker as Clone>::clone(&topic_tracker0).get_topic_nodes(&topic).await?;
    assert!(nodes0.is_empty());
    println!("Query with node_id0 expected to be empty: {nodes0:?}");
    
    sleep(Duration::from_millis(10)).await;

    let nodes1 = <TopicTracker as Clone>::clone(&topic_tracker1).get_topic_nodes(&topic).await?;
    println!("Query with node_id1 expected to contain node_id0: {nodes1:?}");
    assert_eq!(nodes1[0].to_string(),topic_tracker0.node_id.to_string());

    Ok(())
}