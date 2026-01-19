use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use dht::async_dht::AsyncDht;
use ed25519_dalek::SigningKey;
use iroh::EndpointId;
use iroh_gossip::api::{GossipReceiver, GossipSender};
use sha2::Digest;

#[derive(Debug, Clone)]
pub struct TopicDiscoveryConfig {
    pub signing_key: SigningKey,
    pub announce_interval: Duration,
    pub discovery_interval: Duration,
    pub max_peers_per_round: Option<usize>,
}

impl Default for TopicDiscoveryConfig {
    fn default() -> Self {
        Self {
            signing_key: SigningKey::generate(&mut rand::rng()),
            announce_interval: Duration::from_secs(300),
            discovery_interval: Duration::from_secs(60),
            max_peers_per_round: Some(5),
        }
    }
}

impl TopicDiscoveryConfig {
    pub fn new(signing_key: SigningKey) -> Self {
        Self {
            signing_key,
            ..Default::default()
        }
    }

    pub fn announce_interval(mut self, interval: Duration) -> Self {
        self.announce_interval = interval;
        self
    }

    pub fn discovery_interval(mut self, interval: Duration) -> Self {
        self.discovery_interval = interval;
        self
    }

    pub fn max_peers_per_round(mut self, max: Option<usize>) -> Self {
        self.max_peers_per_round = max;
        self
    }
}

#[derive(Debug)]
pub struct TopicDiscoveryHandle {
    running: Arc<AtomicBool>,
    _tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl TopicDiscoveryHandle {
    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }
}

impl Drop for TopicDiscoveryHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

pub trait TopicDiscoveryExt {
    #[allow(async_fn_in_trait)]
    async fn subscribe_with_discovery_joined(
        &self,
        topic_id: Vec<u8>,
        bootstrap_nodes: Vec<EndpointId>,
        config: TopicDiscoveryConfig,
    ) -> anyhow::Result<(GossipSender, GossipReceiver, TopicDiscoveryHandle)>;

    #[allow(async_fn_in_trait)]
    async fn subscribe_with_discovery(
        &self,
        topic_id: Vec<u8>,
        bootstrap_nodes: Vec<EndpointId>,
        config: TopicDiscoveryConfig,
    ) -> anyhow::Result<(GossipSender, GossipReceiver, TopicDiscoveryHandle)>;
}

impl TopicDiscoveryExt for iroh_gossip::net::Gossip {
    async fn subscribe_with_discovery_joined(
        &self,
        topic_id: Vec<u8>,
        bootstrap_nodes: Vec<EndpointId>,
        config: TopicDiscoveryConfig,
    ) -> anyhow::Result<(GossipSender, GossipReceiver, TopicDiscoveryHandle)> {
        let (sender, mut receiver, handle) = self
            .subscribe_with_discovery(topic_id, bootstrap_nodes, config)
            .await?;
        receiver.joined().await?;
        Ok((sender, receiver, handle))
    }

    async fn subscribe_with_discovery(
        &self,
        topic_id: Vec<u8>,
        bootstrap_nodes: Vec<EndpointId>,
        config: TopicDiscoveryConfig,
    ) -> anyhow::Result<(GossipSender, GossipReceiver, TopicDiscoveryHandle)> {
        let topic_bytes = topic_hash_32(&topic_id);
        let (sender, receiver) = self
            .subscribe(
                iroh_gossip::proto::TopicId::from_bytes(topic_bytes),
                bootstrap_nodes,
            )
            .await?
            .split();

        let running = Arc::new(AtomicBool::new(true));
        let tasks = vec![
            spawn_announce_task(
                running.clone(),
                config.signing_key.clone(),
                topic_bytes,
                config.announce_interval,
            ),
            spawn_discovery_task(
                running.clone(),
                sender.clone(),
                config.signing_key.clone(),
                topic_bytes,
                config.discovery_interval,
                config.max_peers_per_round,
            ),
        ];

        let handle = TopicDiscoveryHandle {
            running,
            _tasks: tasks,
        };

        Ok((sender, receiver, handle))
    }
}

async fn init_dht() -> anyhow::Result<AsyncDht> {
    let dht = dht::Dht::builder()
        .extra_bootstrap(&["pkarr.rustonbsd.com:6881", "relay.pkarr.org:6881"])
        .build()?
        .as_async();

    if !dht.bootstrapped().await {
        anyhow::bail!("DHT bootstrap failed");
    }

    Ok(dht)
}

fn spawn_announce_task(
    running: Arc<AtomicBool>,
    signing_key: SigningKey,
    topic_hash_32: [u8; 32],
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut dht: Option<AsyncDht> = None;
        let mut backoff = Duration::from_secs(5);

        while running.load(Ordering::Relaxed) {
            if dht.is_none() {
                match init_dht().await {
                    Ok(d) => dht = Some(d),
                    Err(e) => {
                        tracing::warn!("DHT init failed: {e}, retrying...");
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(Duration::from_secs(60));
                        continue;
                    }
                }
            }

            let Some(ref dht) = dht else { continue };

            let id = match dht::Id::from_bytes(topic_hash_20(&topic_hash_32)) {
                Ok(id) => id,
                Err(e) => {
                    tracing::error!("invalid topic hash: {e}");
                    break;
                }
            };

            match dht.announce_signed_peer(id, &signing_key).await {
                Ok(_) => {
                    tracing::debug!("DHT announce success for topic");
                    backoff = Duration::from_secs(5);
                }
                Err(e) => {
                    tracing::warn!("DHT announce failed: {e}");
                    backoff = (backoff * 2).min(Duration::from_secs(60));
                }
            }

            tokio::time::sleep(interval.max(backoff)).await;
        }
    })
}

fn spawn_discovery_task(
    running: Arc<AtomicBool>,
    gossip_sender: GossipSender,
    signing_key: SigningKey,
    topic_hash_32: [u8; 32],
    interval: Duration,
    max_peers: Option<usize>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut dht: Option<AsyncDht> = None;
        let mut known_peers: HashSet<[u8; 32]> = HashSet::new();

        known_peers.insert(signing_key.verifying_key().to_bytes());

        while running.load(Ordering::Relaxed) {
            if dht.is_none() {
                match init_dht().await {
                    Ok(d) => dht = Some(d),
                    Err(e) => {
                        tracing::warn!("DHT init failed: {e}");
                        tokio::time::sleep(Duration::from_secs(10)).await;
                        continue;
                    }
                }
            }

            let Some(ref dht) = dht else { continue };

            let id = match dht::Id::from_bytes(topic_hash_20(&topic_hash_32)) {
                Ok(id) => id,
                Err(e) => {
                    tracing::error!("Invalid topic hash: {e}");
                    break;
                }
            };

            let peers = collect_peers_with_timeout(dht, id, Duration::from_secs(30)).await;

            let new_key_bytes: Vec<[u8; 32]> = peers
                .into_iter()
                .filter(|key_bytes| !known_peers.contains(key_bytes))
                .take(max_peers.unwrap_or(usize::MAX))
                .collect();

            let new_peers: Vec<EndpointId> = new_key_bytes
                .into_iter()
                .filter_map(|key_bytes| {
                    ed25519_dalek::VerifyingKey::from_bytes(&key_bytes)
                        .ok()
                        .map(|vk| {
                            known_peers.insert(key_bytes);
                            iroh::PublicKey::from_verifying_key(vk)
                        })
                })
                .collect();

            if !new_peers.is_empty() {
                tracing::debug!("Discovered {} new peers for topic", new_peers.len());

                if let Err(e) = gossip_sender.join_peers(new_peers).await {
                    tracing::warn!("Failed to join discovered peers: {e}");
                }
            }

            tokio::time::sleep(interval).await;
        }
    })
}

async fn collect_peers_with_timeout(
    dht: &AsyncDht,
    id: dht::Id,
    timeout: Duration,
) -> Vec<[u8; 32]> {
    use futures_lite::StreamExt;

    let mut stream = dht.get_signed_peers(id).await;
    let mut results = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout;

    while let Ok(Some(items)) = tokio::time::timeout_at(deadline, stream.next()).await {
        for item in items {
            let key_bytes = *item.key();
            results.push(key_bytes);
        }
    }

    results
}

fn topic_hash_32(topic_bytes: &Vec<u8>) -> [u8; 32] {
    let mut hasher = sha2::Sha512::new();
    hasher.update("/iroh/topic-discovery/v2");
    hasher.update(topic_bytes);
    hasher.finalize()[..32].try_into().expect("hashing failed")
}

fn topic_hash_20(topic_hash_32: &[u8; 32]) -> [u8; 20] {
    let mut hasher = sha2::Sha512::new();
    hasher.update(topic_hash_32);
    hasher.finalize()[..20].try_into().expect("hashing failed")
}