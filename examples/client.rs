use std::sync::Arc;

use anyhow::Result;
use iroh::{Endpoint, SecretKey};
use iroh_gossip::proto::TopicId;
use iroh_topic_tracker::topic_tracker::{Topic, TopicTracker};

#[tokio::main]
async fn main() -> Result<()> {
    let topic = Topic::from_passphrase("my test topic");
    let _topic_id: TopicId = topic.clone().into();
    let secret_key =  SecretKey::generate(rand::rngs::OsRng);
    
    let endpoint = Endpoint::builder()
        .secret_key(secret_key)
        .discovery_n0()
        .discovery_dht()
        .bind()
        .await?;
    
    let topic_tracker = Arc::new(TopicTracker::new(&endpoint));
    let _router = iroh::protocol::Router::builder(endpoint.clone())
        .accept(TopicTracker::ALPN, topic_tracker.clone())
        .spawn()
        .await?;

    // Send a request
    let node_ids_for_topic = topic_tracker.get_topic_nodes(&topic).await?;
    println!("Iroh node_ids for topic: {:?}",node_ids_for_topic);

    Ok(())
}