# Iroh Topic Tracker
**Tracker** for Iroh NodeId's in **GossipSub** Topics made easy. The lib comes with a hosted iroh-topic-tracker **BOOTSTRAP_NODE**.

## Try it

Get the last connected NodeId's for a given gossip topic.

    cargo run --example client

Run your own dedicated topic tracker node. 

    cargo run --example server

(adjust the secret.rs SecretKey and change the BOOTSTRAP_NODES public key in topic_tracker.rs (line ~33) -> impl TopicTracker)

## Build server for release

    cargo build --release --example server

## Lib usage
See examples but tldr:

### Example Minimal
```rust
use iroh_topic_tracker::topic_tracker::TopicTracker;

let topic = Topic::from_passphrase("my topic name");
let topic_tracker = TopicTracker::new(&endpoint);

topic_tracker.get_topic_nodes(&topic).await?;
```

### Example Minimally more
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
