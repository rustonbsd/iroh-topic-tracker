mod iroh_gossip_int;

#[cfg(feature="iroh-gossip-auto-discovery")]
pub use iroh_gossip_int::{AutoDiscoveryGossip,AutoDiscoveryBuilder};