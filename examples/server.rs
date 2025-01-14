use std::{str::FromStr, sync::Arc};

use anyhow::Result;
use iroh::{Endpoint, SecretKey};
use iroh_topic_tracker::{secret::SECRET_SERVER_KEY, topic_tracker::TopicTracker};
use tokio::io::{AsyncBufReadExt, BufReader};

#[tokio::main]
async fn main() -> Result<()> {

    let secret_key =  SecretKey::from_str(SECRET_SERVER_KEY)?;
    
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

    let from = tokio::io::stdin();
    let mut lines = BufReader::new(from).lines();
    while let Some(_) = lines.next_line().await? {
        println!("{:?}",topic_tracker.memory_footprint().await);
    }
    
    Ok(())
}