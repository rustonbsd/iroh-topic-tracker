# Iroh Topic Tracker

[![Crates.io](https://img.shields.io/crates/v/iroh-topic-tracker.svg)](https://crates.io/crates/iroh-topic-tracker)
[![Docs.rs](https://docs.rs/iroh-topic-tracker/badge.svg)](https://docs.rs/iroh-topic-tracker)

**An easy-to-use tracker for Iroh NodeId's in GossipSub topics.** This library includes a hosted `iroh-topic-tracker` **BOOTSTRAP_NODE** to facilitate seamless tracking.

---

## Getting Started

### Automatic Discovery with iroh-gossip

This crate comes with an [iroh-gossip](https://crates.io/crates/iroh-gossip) integration, available via the feature flag:
```feature = ["iroh-gossip-auto-discovery"]```

```rust
use iroh_topic_tracker::integrations::iroh_gossip::*;

let endpoint = Endpoint::builder().bind().await?;

// Setup Gossip with auto discovery
let gossip = Gossip::new(endpoint.clone()).await?;
// ::new() or ::builder().spawn_with_auto_discovery()
let gossip = Gossip::builder()
    .spawn_with_auto_discovery(endpoint.clone())
    .await?;

let router = iroh::protocol::Router::builder(endpoint.clone())
    .accept(iroh_gossip::ALPN, gossip.gossip.clone())
    .spawn().await?;

// Gossip Topic
let topic = Topic::from_passphrase("my-iroh-gossip-topic");

// Subscribe and join with automatic peer discovery
let (sink, mut stream) = gossip.subscribe_and_join(topic.into()).await?.split();
```

---

### Try It Out

1. **Get the last connected NodeId's for a given gossip topic:**
    ```bash
    cargo run --example client
    ```

2. **Run your own dedicated topic tracker node:**
    ```bash
    cargo run --example server
    ```
   *Note: Adjust the `secret.rs` SecretKey to ensure secure communication, and update the `BOOTSTRAP_NODES` public key in `topic_tracker.rs` (around line 33) to correctly point to the desired bootstrap node for discovery.*

---

### Build Server for Release

To build the server in release mode:
```bash
cargo build --release --example server
```

---

## Library Usage

Refer to the examples below for quick guidance:

### Basic Example

```rust
use iroh_topic_tracker::topic_tracker::TopicTracker;

let topic = Topic::from_passphrase("my topic name");
let topic_tracker = TopicTracker::new(&endpoint);

topic_tracker.get_topic_nodes(&topic).await?;
```

### Expanded Example

```rust
use iroh_topic_tracker::topic_tracker::TopicTracker;

let topic = Topic::from_passphrase("my test topic");
let endpoint = Endpoint::builder()
    .secret_key(SecretKey::generate(rand::rngs::OsRng))
    .discovery_n0()
    .discovery_dht()
    .bind()
    .await?;

let topic_tracker = TopicTracker::new(&endpoint);
let router = Router::builder(endpoint.clone())
    .accept(TopicTracker::ALPN, topic_tracker.clone())
    .spawn()
    .await?;

topic_tracker.get_topic_nodes(&topic).await?;
```

