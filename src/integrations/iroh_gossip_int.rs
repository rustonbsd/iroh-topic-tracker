use std::sync::Arc;

use anyhow::Result;
use iroh::Endpoint;
use iroh_gossip::{
    net::{Builder, Gossip, GossipTopic},
    proto::TopicId,
};

use crate::topic_tracker::TopicTracker;

/// Auto-discovery enabled Gossip wrapper
#[derive(Debug, Clone)]
pub struct GossipAutoDiscovery {
    pub gossip: Arc<Gossip>,
    topic_tracker: Arc<TopicTracker>,
}

/// Trait for creating new auto-discovery enabled Gossip instances
pub trait AutoDiscoveryNew {
    /// Creates a new GossipAutoDiscovery from the given endpoint
    ///
    /// # Arguments
    ///
    /// * `endpoint` - The network endpoint to use
    ///
    /// # Returns
    ///
    /// A Result containing the new GossipAutoDiscovery instance
    #[allow(async_fn_in_trait)]
    async fn new(endpoint: Endpoint) -> Result<GossipAutoDiscovery>;
}

impl AutoDiscoveryNew for Gossip {
    /// Creates a new GossipAutoDiscovery from a Gossip instance
    ///
    /// # Arguments
    ///
    /// * `endpoint` - The network endpoint to use
    ///
    /// # Returns
    ///
    /// A Result containing the new GossipAutoDiscovery instance
    async fn new(endpoint: Endpoint) -> Result<GossipAutoDiscovery> {
        Gossip::builder().spawn_with_auto_discovery(endpoint).await
    }
}

/// Trait for building auto-discovery enabled Gossip instances
pub trait AutoDiscoveryBuilder {
    /// Spawns a new GossipAutoDiscovery from a Builder
    ///
    /// # Arguments
    ///
    /// * `endpoint` - The network endpoint to use
    ///
    /// # Returns
    /// 
    /// A Result containing the new GossipAutoDiscovery instance
    #[allow(async_fn_in_trait)]
    async fn spawn_with_auto_discovery(self, endpoint: Endpoint) -> Result<GossipAutoDiscovery>;
}

impl AutoDiscoveryBuilder for Builder {
    /// Spawns a new GossipAutoDiscovery from this Builder
    ///
    /// # Arguments
    ///
    /// * `endpoint` - The network endpoint to use
    ///
    /// # Returns
    ///
    /// A Result containing the new GossipAutoDiscovery instance
    async fn spawn_with_auto_discovery(self, endpoint: Endpoint) -> Result<GossipAutoDiscovery> {
        let topic_tracker = Arc::new(TopicTracker::new(&endpoint));
        let gossip = Arc::new(self.spawn(endpoint).await?);

        Ok(GossipAutoDiscovery {
            gossip,
            topic_tracker,
        })
    }
}

/// Trait for auto-discovery enabled Gossip functionality
pub trait AutoDiscoveryGossip {
    /// Subscribes to a topic and joins its network
    ///
    /// # Arguments
    ///
    /// * `topic_id` - The ID of the topic to subscribe to
    ///
    /// # Returns
    ///
    /// A Result containing the GossipTopic
    #[allow(async_fn_in_trait)]
    async fn subscribe_and_join(&self, topic_id: TopicId) -> Result<GossipTopic>;
}

impl AutoDiscoveryGossip for GossipAutoDiscovery {
    /// Subscribes to a topic and joins its network
    ///
    /// # Arguments
    ///
    /// * `topic_id` - The ID of the topic to subscribe to
    ///
    /// # Returns
    ///
    /// A Result containing the GossipTopic
    async fn subscribe_and_join(&self, topic_id: TopicId) -> Result<GossipTopic> {
        let node_ids = self
            .topic_tracker
            .clone()
            .get_topic_nodes(&topic_id.into())
            .await?;

        self.gossip.subscribe_and_join(topic_id, node_ids).await
    }
}
