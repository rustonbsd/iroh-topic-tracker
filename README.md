# iroh-topic-tracker

## No More Bootstrap Server
**IMPORTANT:** A new, serverless alternative is now available (see [distributed-topic-tracker](https://github.com/rustonbsd/distributed-topic-tracker)). You can still self host this project but the Server that I have been hosting for the past months is now (as of the 25.09.2025) offline and the default bootstrap node no longer works. If you need help setting it up on your own, please feel free to reach out or create an issue.

---

[![Crates.io](https://img.shields.io/crates/v/iroh-topic-tracker.svg)](https://crates.io/crates/iroh-topic-tracker)
[![Docs.rs](https://docs.rs/iroh-topic-tracker/badge.svg)](https://docs.rs/iroh-topic-tracker)

**A tracker for Iroh NodeId's in GossipSub topics.**

This library integrates with
[`iroh-gossip`](https://crates.io/crates/iroh-gossip) to automate peer
discovery and includes a hosted `BOOTSTRAP_NODE` for seamless topic tracking
without you needing to host anything. Your peers can discover each other even
if both are behind NATs.

---

## Overview

The crate provides a
[`TopicTracker`](https://docs.rs/iroh-topic-tracker/latest/iroh_topic_tracker/topic_tracker/struct.TopicTracker.html)
to manage and discover peers participating in shared GossipSub topics. It
leverages Iroh's direct connectivity and
[`Router`](https://docs.rs/iroh/latest/iroh/protocol/struct.Router.html) to
handle protocol routing.

### Features

- Automatic peer discovery via `iroh-gossip` (enabled with
  `iroh-gossip-auto-discovery` feature).
- Dedicated bootstrap node support for topic tracking.
- Simple API to fetch active peers for a topic.

---

## Getting Started

### Prerequisites

Add the crate to your `Cargo.toml` with the required features:

```toml
[dependencies]
iroh-topic-tracker = { version = "0.1", features = ["iroh-gossip-auto-discovery"] }
```

### Automatic Discovery

Enable `iroh-gossip` integration to automate peer discovery for topics:

```rust
use futures_lite::StreamExt;
use iroh::Endpoint;
use iroh_gossip::net::{Event, Gossip, GossipEvent};
use iroh_topic_tracker::{integrations::iroh_gossip::*, topic_tracker::Topic};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Configure Iroh endpoint with discovery
    let endpoint = Endpoint::builder().discovery_n0().bind().await?;

    // Configure gossip protocol with auto-discovery
    let gossip = Gossip::builder()
        .spawn_with_auto_discovery(endpoint.clone())
        .await?;

    // Join a topic and start tracking
    let topic = Topic::from_passphrase("my-iroh-gossip-topic");
    let (sink, mut stream) = gossip.subscribe_and_join(topic.into()).await?.split();

    // Read from stream ..
    while let Some(event) = stream.next().await {
        if let Ok(Event::Gossip(GossipEvent::Received(msg))) = event {

            // Do something with msg...
            let msg_text = String::from_utf8(msg.content.to_vec()).unwrap();

        }
    }

    // .. or Send to Sink
    sink.broadcast("my msg goes here".into()).await.unwrap();

    Ok(())
}
```

### Basic Setup with Iroh

```rust
use iroh::{protocol::Router, Endpoint};
use iroh_topic_tracker::topic_tracker::{Topic, TopicTracker};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Configure Iroh endpoint with discovery
    let endpoint = Endpoint::builder().discovery_n0().bind().await?;

    // Initialize topic tracker
    let topic_tracker = TopicTracker::new(&endpoint);

    // Attach to Iroh router
    let router = Router::builder(endpoint.clone())
        .accept(TopicTracker::ALPN, topic_tracker.clone())
        .spawn()
        .await?;

    // Track peers in a topic
    let topic = Topic::from_passphrase("my-secret-topic");
    let peers = topic_tracker.get_topic_nodes(&topic).await?;

    Ok(())
}
```

---

## Examples

### Run a Topic Tracker Node

Start a dedicated tracker node:

```bash
cargo run --example server
```

*Note: Update `secret.rs` with your `SecretKey` and configure `BOOTSTRAP_NODES`
in `topic_tracker.rs` to use your own tracker.*

### Query Active Peers

Fetch the latest NodeIds for a topic:

```bash
cargo run --example client
```

---

## Building

Optimized release build for the tracker server:

```bash
cargo build --release --example server
```

---

## License

This project is licensed under either of

- Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE.txt) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT.txt) or
  <http://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless explicitly stated, any contribution intentionally submitted for
inclusion in this project shall be dual-licensed as above, without any
additional terms or conditions.
