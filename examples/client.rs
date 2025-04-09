use anyhow::Result;
use iroh::{Endpoint, SecretKey};
use iroh_gossip::proto::TopicId;
use iroh_topic_tracker::topic_tracker::{Topic, TopicTracker};

#[tokio::main]
async fn main() -> Result<()> {
    // Create a topic from a passphrase and get its ID
    let topic = Topic::from_passphrase("my test topic");
    let _topic_id: TopicId = topic.clone().into();

    // Generate a new secret key for secure communication
    let secret_key = SecretKey::generate(rand::rngs::ThreadRng::default());

    // Configure and initialize the network endpoint
    let endpoint = Endpoint::builder()
        .secret_key(secret_key)
        .discovery_n0() // Enable node discovery
        .bind()
        .await?;

    // Create a shared topic tracker instance
    let topic_tracker = TopicTracker::new(&endpoint);

    // Set up the protocol router with topic tracking capability
    let _router = iroh::protocol::Router::builder(endpoint.clone())
        .accept(TopicTracker::ALPN, topic_tracker.clone())
        .spawn()
        .await?;

    // Query nodes participating in this topic
    let node_ids_for_topic = topic_tracker.get_topic_nodes(&topic).await?;
    println!("Iroh node_ids for topic: {:?}", node_ids_for_topic);

    Ok(())
}

