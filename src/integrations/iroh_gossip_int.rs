use std::sync::Arc;

use anyhow::Result;
use iroh::Endpoint;
use iroh_gossip::{
    net::{Builder, Gossip, GossipTopic},
    proto::TopicId,
};

use crate::topic_tracker::TopicTracker;

#[derive(Debug, Clone)]
pub struct GossipAutoDiscovery {
    pub gossip: Arc<Gossip>,
    topic_tracker: Arc<TopicTracker>,
}

pub trait AutoDiscoveryBuilder {
    #[allow(async_fn_in_trait)]
    async fn spawn_with_auto_discovery(self, endpoint: Endpoint) -> Result<GossipAutoDiscovery>;
}

impl AutoDiscoveryBuilder for Builder {
    async fn spawn_with_auto_discovery(self, endpoint: Endpoint) -> Result<GossipAutoDiscovery> {
        let topic_tracker = Arc::new(TopicTracker::new(&endpoint));
        let gossip = Arc::new(self.spawn(endpoint).await?);

        Ok(GossipAutoDiscovery {
            gossip,
            topic_tracker,
        })
    }
}

pub trait AutoDiscoveryGossip {
    #[allow(async_fn_in_trait)]
    async fn subscribe_and_join(&self, topic_id: TopicId) -> Result<GossipTopic>;
}

impl AutoDiscoveryGossip for GossipAutoDiscovery {
    async fn subscribe_and_join(&self, topic_id: TopicId) -> Result<GossipTopic> {
        let node_ids = self
            .topic_tracker
            .clone()
            .get_topic_nodes(&topic_id.into())
            .await?;
        
        self.gossip.subscribe_and_join(topic_id, node_ids).await
    }
}
