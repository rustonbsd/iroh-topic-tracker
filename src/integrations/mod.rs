mod iroh_gossip_int;

#[cfg(feature = "iroh-gossip-auto-discovery")]
pub mod iroh_gossip {
    pub use super::iroh_gossip_int::{AutoDiscoveryBuilder, AutoDiscoveryGossip, AutoDiscoveryNew,GossipAutoDiscovery};
}
