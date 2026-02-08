use std::{
    collections::{BTreeSet, HashMap},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use dht::async_dht::AsyncDht;
use ed25519_dalek::SigningKey;
use futures_lite::StreamExt;
use iroh::{
    EndpointId, Watcher,
    endpoint::{ConnectionInfo, EndpointHooks, PathInfoList},
};
use iroh_gossip::api::{GossipReceiver, GossipSender};
use n0_future::time;
use n0_watcher::Watchable;
use sha2::Digest;
use tokio::time::{sleep, timeout};

#[derive(Debug, Clone)]
pub struct TopicDiscoveryConfig {
    pub signing_key: SigningKey,
    /// EndpointHook to track connections
    pub hook: TopicDiscoveryHook,
    /// How often to re-announce to DHT (default: 5 minutes)
    pub announce_interval: Duration,
    /// Discovery interval when we have peers (default: 60s)
    pub discovery_interval: Duration,
    /// Discovery interval when we have no peers (default: 2s) - aggressive mode
    pub discovery_interval_no_peers: Duration,
    /// Timeout for individual connection attempts (default: 5s)
    pub connection_timeout: Duration,
    /// How long before we retry a failed peer (default: 5 minutes)
    pub retry_interval: Duration,
    /// Max peers to attempt per discovery round (default: 5)
    pub max_peers_per_round: Option<usize>,
}

impl TopicDiscoveryConfig {
    pub fn new(signing_key: SigningKey, hook: TopicDiscoveryHook) -> Self {
        Self {
            signing_key,
            hook,
            announce_interval: Duration::from_secs(300),
            discovery_interval: Duration::from_secs(60),
            discovery_interval_no_peers: Duration::from_secs(2),
            connection_timeout: Duration::from_secs(5),
            retry_interval: Duration::from_secs(300),
            max_peers_per_round: Some(5),
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

    pub fn discovery_interval_no_peers(mut self, interval: Duration) -> Self {
        self.discovery_interval_no_peers = interval;
        self
    }

    pub fn connection_timeout(mut self, timeout: Duration) -> Self {
        self.connection_timeout = timeout;
        self
    }

    pub fn retry_interval(mut self, interval: Duration) -> Self {
        self.retry_interval = interval;
        self
    }

    pub fn max_peers_per_round(mut self, max: Option<usize>) -> Self {
        self.max_peers_per_round = max;
        self
    }
}

#[derive(Debug, Clone)]
struct DiscoveryState {
    /// Number of peers we've successfully joined to gossip
    connected_count: Watchable<usize>,
    /// Set of connected peer IDs (can contain old/stale entries)
    connected_peers: Arc<Mutex<BTreeSet<EndpointId>>>,
    /// Signal to stop all tasks
    stopped: Arc<AtomicBool>,
    /// Peers we've attempted to connect to, with timestamp for retry logic
    attempted: Arc<Mutex<HashMap<[u8; 32], Instant>>>,
    /// How long before we retry a failed peer
    retry_interval: Duration,
}

#[derive(Debug, Clone)]
pub struct TopicDiscoveryHook {
    states: Arc<Mutex<Vec<DiscoveryState>>>,
}

impl Default for TopicDiscoveryHook {
    fn default() -> Self {
        Self::new()
    }
}

impl TopicDiscoveryHook {
    pub fn new() -> Self {
        Self {
            states: Arc::new(Mutex::new(vec![])),
        }
    }

    fn add_state(&self, state: &DiscoveryState) {
        if let Ok(mut states) = self.states.lock() {
            states.push(state.clone());
        }
    }

    fn increment_connected(&self, conn_info: &ConnectionInfo) {
        let states = self.states.clone();
        let peer_id = conn_info.remote_id();
        let init_rx = udp_rx_bytes(conn_info.paths());
        let paths = conn_info.paths();
        tracing::debug!(
            "TopicDiscoveryHook: connection {} established",
            peer_id.fmt_short()
        );
        tokio::spawn(async move {
            match timeout(Duration::from_secs(10), async {
                while init_rx == udp_rx_bytes(paths.clone()) {
                    sleep(Duration::from_millis(100)).await;
                }
            })
            .await
            {
                Ok(_) => {
                    tracing::info!(
                        "TopicDiscoveryHook: connection {} shows UDP activity, incrementing state, init={}, after={}",
                        peer_id.fmt_short(),
                        init_rx,
                        udp_rx_bytes(paths.clone())
                    );
                    if let Ok(states) = states.lock() {
                        for state in states.iter() {
                            state.increment_connected(peer_id);
                        }
                    }
                }
                Err(_) => tracing::info!(
                    "TopicDiscoveryHook: connection {} did not show UDP activity within timeout, skipping state increment",
                    peer_id.fmt_short()
                ),
            }
        });
    }
}

fn udp_rx_bytes(
    mut paths: impl Watcher<Value = PathInfoList> + Unpin + Send + Sync + 'static,
) -> u64 {
    paths
        .get()
        .iter()
        .map(|path| path.stats().udp_rx.bytes)
        .reduce(|a, b| a + b)
        .unwrap_or_default()
}

impl EndpointHooks for TopicDiscoveryHook {
    async fn after_handshake<'a>(
        &'a self,
        conn: &'a iroh::endpoint::ConnectionInfo,
    ) -> iroh::endpoint::AfterHandshakeOutcome {
        self.increment_connected(conn);
        iroh::endpoint::AfterHandshakeOutcome::accept()
    }
}

impl DiscoveryState {
    fn new(retry_interval: Duration) -> Arc<Self> {
        Arc::new(Self {
            connected_count: Watchable::new(0),
            connected_peers: Arc::new(Mutex::new(BTreeSet::new())),
            stopped: Arc::new(AtomicBool::new(false)),
            attempted: Arc::new(Mutex::new(HashMap::new())),
            retry_interval,
        })
    }

    fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }

    fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Relaxed)
    }

    fn has_connections(&self) -> bool {
        self.connected_count.get() > 0
    }

    fn connection_count(&self) -> usize {
        self.connected_count.get()
    }

    fn increment_connected(&self, peer_id: EndpointId) {
        if let Ok(mut peers) = self.connected_peers.lock()
            && peers.insert(peer_id)
        {
            self.connected_count
                .set(self.connected_count.get() + 1)
                .ok();
        }
    }

    /// Mark a peer as attempted. Returns true if we should try to connect
    /// (either new peer, or retry interval has elapsed).
    fn should_attempt(&self, peer: [u8; 32]) -> bool {
        if let Ok(mut map) = self.attempted.lock() {
            match map.get(&peer) {
                None => {
                    map.insert(peer, Instant::now());
                    true
                }
                Some(last_attempt) => {
                    if last_attempt.elapsed() > self.retry_interval {
                        map.insert(peer, Instant::now());
                        true
                    } else {
                        false
                    }
                }
            }
        } else {
            false
        }
    }

    fn reset_attempt(&self, peer: [u8; 32]) {
        if let Ok(mut map) = self.attempted.lock() {
            map.remove(&peer);
        }
    }
}

#[derive(Debug)]
pub struct TopicDiscoveryHandle {
    state: Arc<DiscoveryState>,
    _tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl TopicDiscoveryHandle {
    pub fn stop(&self) {
        self.state.stop();
    }

    pub fn is_running(&self) -> bool {
        !self.state.is_stopped()
    }

    pub fn has_connections(&self) -> bool {
        self.state.has_connections()
    }

    pub fn connection_count(&self) -> usize {
        self.state.connection_count()
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
        tracing::info!("subscribe_with_discovery_joined: starting subscription");
        let (sender, mut receiver, handle) = self
            .subscribe_with_discovery(topic_id, bootstrap_nodes, config)
            .await?;
        tracing::info!("subscribe_with_discovery_joined: waiting for receiver.joined()");
        receiver.joined().await?;
        tracing::info!("subscribe_with_discovery_joined: joined successfully");
        Ok((sender, receiver, handle))
    }

    async fn subscribe_with_discovery(
        &self,
        topic_id: Vec<u8>,
        bootstrap_nodes: Vec<EndpointId>,
        config: TopicDiscoveryConfig,
    ) -> anyhow::Result<(GossipSender, GossipReceiver, TopicDiscoveryHandle)> {
        tracing::info!("subscribe_with_discovery: computing topic hash");
        let topic_bytes = topic_hash_32(&topic_id);
        tracing::debug!(
            "subscribe_with_discovery: topic_hash={}",
            hex::encode(topic_bytes)
        );

        tracing::info!("subscribe_with_discovery: subscribing to gossip topic");
        let (sender, receiver) = self
            .subscribe(
                iroh_gossip::proto::TopicId::from_bytes(topic_bytes),
                bootstrap_nodes,
            )
            .await?
            .split();

        tracing::info!(
            "subscribe_with_discovery: subscribed, spawning announce and discovery tasks"
        );

        let state = DiscoveryState::new(config.retry_interval);
        config.hook.add_state(&state);

        tracing::info!("subscribe_with_discovery: initializing shared DHT");
        let mut tries = 0;
        let dht = loop {
            if let Ok(dht) = init_dht().await {
                break Arc::new(dht);
            }
            tracing::warn!("subscribe_with_discovery: DHT init failed, retrying in 2s");
            tokio::time::sleep(Duration::from_secs(2)).await;
            tries += 1;
            if tries > 5 {
                anyhow::bail!("DHT init failed after 5 attempts");
            }
        };

        let tasks = vec![
            spawn_announce_task(state.clone(), dht.clone(), topic_bytes, config.clone()),
            spawn_discovery_task(state.clone(), dht, sender.clone(), topic_bytes, config),
        ];

        let handle = TopicDiscoveryHandle {
            state,
            _tasks: tasks,
        };

        Ok((sender, receiver, handle))
    }
}

async fn init_dht() -> anyhow::Result<AsyncDht> {
    tracing::info!("init_dht: building DHT with bootstrap nodes");
    let dht = dht::Dht::builder()
        .no_bootstrap()
        .bootstrap(&["pkarr.rustonbsd.com:6881", "relay.pkarr.org:6881"])
        .build()?
        .as_async();

    tracing::info!("init_dht: waiting for DHT bootstrap... ");
    if !dht.bootstrapped().await {
        tracing::error!("init_dht: DHT bootstrap failed");
        anyhow::bail!("DHT bootstrap failed");
    }

    Ok(dht)
}

fn spawn_connector(
    state: Arc<DiscoveryState>,
    gossip_sender: GossipSender,
    peer: EndpointId,
    timeout: Duration,
) {
    tokio::spawn(async move {
        if state.is_stopped() {
            return;
        }

        tracing::debug!("connector: joining peer {} via gossip", peer.fmt_short());

        let _ = gossip_sender.join_peers(vec![peer]).await;

        if state.is_stopped() {
            return;
        }

        let wait_for_connection = async {
            let mut stream = state.connected_count.watch().stream();
            while let Some(next_counter) = stream.next().await {
                if next_counter > 0 {
                    return true;
                }
            }
            false
        };

        match time::timeout(timeout, wait_for_connection).await {
            Ok(true) => {
                state.increment_connected(peer);
            }
            Ok(false) => {
                tracing::debug!(
                    "connector: stopped while waiting for connection to {}",
                    peer.fmt_short()
                );

                if !state.has_connections() {
                    state.reset_attempt(*peer.as_bytes());
                }
            }
            Err(_) => {
                tracing::warn!(
                    "connector: timeout waiting for connection to {} after {:?}",
                    peer.fmt_short(),
                    timeout
                );
                if !state.has_connections() {
                    state.reset_attempt(*peer.as_bytes());
                }
            }
        }
    });
}

fn spawn_announce_task(
    state: Arc<DiscoveryState>,
    dht: Arc<AsyncDht>,
    topic_hash_32: [u8; 32],
    config: TopicDiscoveryConfig,
) -> tokio::task::JoinHandle<()> {
    tracing::info!("spawn_announce_task: starting announce task");
    tokio::spawn(async move {
        let mut backoff = Duration::from_secs(5);
        let mut round = 0u64;

        let id = match dht::Id::from_bytes(topic_hash_20(&topic_hash_32)) {
            Ok(id) => id,
            Err(e) => {
                tracing::error!("announce_task: invalid topic hash: {e}");
                return;
            }
        };

        while !state.is_stopped() {
            round += 1;
            tracing::debug!("announce_task: round {round} starting");

            tracing::debug!("announce_task: announcing to DHT");
            match dht.announce_signed_peer(id, &config.signing_key).await {
                Ok(_) => {
                    tracing::info!("announce_task: DHT announce success");
                    backoff = Duration::from_secs(5);
                    tracing::debug!("announce_task: sleeping for {:?}", config.announce_interval);
                    tokio::time::sleep(config.announce_interval).await;
                }
                Err(e) => {
                    tracing::warn!(
                        "announce_task: DHT announce failed: {e}, retrying in {backoff:?}"
                    );

                    // Token staleness fix: Do a fresh GET to acquire new tokens before retry.
                    // The PUT fails with NoClosestNodes when tokens expire (5min rotation).
                    // get_signed_peers forces the DHT to issue fresh tokens for our IP I think?!
                    tracing::debug!("announce_task: refreshing tokens via get_signed_peers");
                    let mut stream = dht.get_signed_peers(id).await;
                    let _ = stream.next().await;

                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(60));
                }
            }
        }
        tracing::info!("announce_task: stopped");
    })
}

fn spawn_discovery_task(
    state: Arc<DiscoveryState>,
    dht: Arc<AsyncDht>,
    gossip_sender: GossipSender,
    topic_hash_32: [u8; 32],
    config: TopicDiscoveryConfig,
) -> tokio::task::JoinHandle<()> {
    let my_key = config.signing_key.verifying_key().to_bytes();

    tracing::info!("spawn_discovery_task: starting discovery task");
    tokio::spawn(async move {
        let mut round = 0u64;

        let mut no_peer_backoff = config.discovery_interval_no_peers;
        let backoff_increment = config.discovery_interval_no_peers;

        let id = match dht::Id::from_bytes(topic_hash_20(&topic_hash_32)) {
            Ok(id) => id,
            Err(e) => {
                tracing::error!("discovery_task: invalid topic hash: {e}");
                return;
            }
        };

        while !state.is_stopped() {
            round += 1;
            tracing::debug!(
                "discovery_task: round {round} starting (connected: {}, backoff: {:?})",
                state.connection_count(),
                no_peer_backoff
            );

            tracing::debug!("discovery_task: querying DHT for peers");
            let peers = collect_peers_with_timeout(
                &dht,
                id,
                Duration::from_secs(30),
                config.announce_interval,
            )
            .await;
            tracing::debug!("discovery_task: found {} peers from DHT", peers.len());

            let mut spawned = 0;
            for key_bytes in peers
                .iter()
                .take(config.max_peers_per_round.unwrap_or(usize::MAX))
            {
                if *key_bytes == my_key {
                    continue;
                }

                if !state.should_attempt(*key_bytes) {
                    continue;
                }

                let Some(peer) = ed25519_dalek::VerifyingKey::from_bytes(key_bytes)
                    .ok()
                    .map(iroh::PublicKey::from_verifying_key)
                else {
                    continue;
                };

                spawn_connector(
                    state.clone(),
                    gossip_sender.clone(),
                    peer,
                    config.connection_timeout,
                );
                spawned += 1;
            }

            if spawned > 0 {
                tracing::info!("discovery_task: spawned {spawned} connector tasks");
            }

            let interval = if state.has_connections() {
                no_peer_backoff = config.discovery_interval_no_peers;
                config.discovery_interval
            } else {
                let current = no_peer_backoff;
                no_peer_backoff =
                    (no_peer_backoff + backoff_increment).min(config.discovery_interval);
                current
            };

            tracing::debug!(
                "discovery_task: sleeping for {:?} (has_connections: {}, next_backoff: {:?})",
                interval,
                state.has_connections(),
                no_peer_backoff
            );
            tokio::time::sleep(interval).await;
        }
        tracing::info!("discovery_task: stopped");
    })
}

async fn collect_peers_with_timeout(
    dht: &AsyncDht,
    id: dht::Id,
    timeout: Duration,
    announce_interval: Duration,
) -> Vec<[u8; 32]> {
    use futures_lite::StreamExt;

    tracing::debug!("collect_peers_with_timeout: starting peer collection");
    let mut stream = dht.get_signed_peers(id).await;
    let deadline = tokio::time::Instant::now() + timeout;
    let mut valid_items = Vec::new();
    while let Ok(Some(items)) = tokio::time::timeout_at(deadline, stream.next()).await {
        tracing::debug!(
            "collect_peers_with_timeout: received batch of {} signed peers from DHT",
            items.len()
        );
        for item in items {
            let key_hex = hex::encode(&item.key()[..8]); // First 8 bytes for brevity
            tracing::debug!(
                "collect_peers_with_timeout: peer key={key_hex}... timestamp={}",
                item.timestamp()
            );

            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_micros();

            let max_age = announce_interval.as_micros() + 10_000_000; // announce + 10s buffer
            let age = now.saturating_sub(item.timestamp() as u128);
            if age > max_age {
                tracing::debug!(
                    "collect_peers_with_timeout: skipping stale peer {key_hex}... (age: {}ms, max: {}ms)",
                    age / 1000,
                    max_age / 1000
                );
                continue;
            }

            if !valid_items.contains(&item) {
                valid_items.push(item);
            }
        }
    }

    valid_items.sort_by_key(|item| item.timestamp());
    valid_items.reverse();
    valid_items.dedup_by_key(|item| item.key().to_vec());

    tracing::debug!(
        "collect_peers_with_timeout: finished with {} peers",
        valid_items.len()
    );
    valid_items.iter().map(|item| *item.key()).collect()
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
