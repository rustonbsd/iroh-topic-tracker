use std::{str::FromStr, sync::Arc};

use anyhow::Result;
use iroh::{Endpoint, SecretKey};
use iroh_topic_tracker::{secret::SECRET_SERVER_KEY, topic_tracker::TopicTracker};
use tokio::io::{AsyncBufReadExt, BufReader};

#[tokio::main]
async fn main() -> Result<()> {
    // Create secret key from constant
    let secret_key = SecretKey::from_str(SECRET_SERVER_KEY)?;
    
    // Configure and initialize network endpoint
    let endpoint = Endpoint::builder()
        .secret_key(secret_key)
        .discovery_n0()
        .bind()
        .await?;

    // Initialize topic tracker and wrap in Arc for sharing
    let topic_tracker = Arc::new(TopicTracker::new(&endpoint).spawn_optional().await?);

    // Set up router to handle topic tracking protocol
    let _router = iroh::protocol::Router::builder(endpoint.clone())
        .accept(TopicTracker::ALPN, topic_tracker.clone())
        .spawn();

    // Read from stdin and print topic tracker memory usage on each line
    let from = tokio::io::stdin();
    let mut lines = BufReader::new(from).lines();
    while let Some(_) = lines.next_line().await? {
        println!("{:?}",topic_tracker.memory_footprint().await);
    }
    
    Ok(())
}