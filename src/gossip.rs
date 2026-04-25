use std::{
    collections::{BTreeSet, HashMap},
    sync::{
        Arc,
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
use tokio::{
    sync::Mutex,
    time::{sleep, timeout},
};

#[derive(Debug, Clone)]
pub struct TopicDiscoveryConfig {
    signing_key: SigningKey,
    /// EndpointHook to track connections
    hook: TopicDiscoveryHook,
    /// How often to re-announce to DHT (default: 5 minutes)
    announce_interval: Duration,
    /// Discovery interval when we have peers (default: 60s)
    discovery_interval: Duration,
    /// Discovery interval after we first started connecting for first_connected_duration (default: 10s)
    first_connected_duration: Option<Duration>,
    /// Discovery interval after we first connected for first_connected_duration (default: 5s)
    discovery_interval_first_connected: Duration,
    /// Discovery interval when we have no peers (default: 2s) - aggressive mode
    discovery_interval_no_peers: Duration,
    /// Timeout for individual connection attempts (default: 5s)
    connection_timeout: Duration,
    /// How long before we retry a failed peer (default: 5 minutes)
    retry_interval: Duration,
    /// Max peers to attempt per discovery round (default: 5)
    max_peers_per_round: Option<usize>,
    /// DHT initialization retry count if None infinite retries
    dht_retries: Option<usize>,
}

pub struct ConfigBuilder(TopicDiscoveryConfig);

impl ConfigBuilder {
    pub fn announce_interval(mut self, interval: Duration) -> Self {
        self.0.announce_interval = interval;
        self
    }

    pub fn discovery_interval(mut self, interval: Duration) -> Self {
        self.0.discovery_interval = interval;
        self
    }

    pub fn discovery_interval_no_peers(mut self, interval: Duration) -> Self {
        self.0.discovery_interval_no_peers = interval;
        self
    }

    pub fn discovery_interval_first_connected(mut self, interval: Duration) -> Self {
        self.0.discovery_interval_first_connected = interval;
        self
    }

    pub fn first_connected_duration(mut self, duration: Option<Duration>) -> Self {
        self.0.first_connected_duration = duration;
        self
    }

    pub fn connection_timeout(mut self, timeout: Duration) -> Self {
        self.0.connection_timeout = timeout;
        self
    }

    pub fn retry_interval(mut self, interval: Duration) -> Self {
        self.0.retry_interval = interval;
        self
    }

    pub fn max_peers_per_round(mut self, max: Option<usize>) -> Self {
        self.0.max_peers_per_round = max;
        self
    }

    pub fn dht_retries(mut self, retries: Option<usize>) -> Self {
        self.0.dht_retries = retries;
        self
    }

    pub fn build(&self) -> TopicDiscoveryConfig {
        self.0.clone()
    }
}

impl TopicDiscoveryConfig {
    pub fn builder(signing_key: SigningKey, hook: TopicDiscoveryHook) -> ConfigBuilder {
        ConfigBuilder(Self {
            signing_key,
            hook,
            announce_interval: Duration::from_secs(300),
            discovery_interval: Duration::from_secs(60),
            first_connected_duration: Some(Duration::from_secs(60)),
            discovery_interval_first_connected: Duration::from_secs(5),
            discovery_interval_no_peers: Duration::from_secs(2),
            connection_timeout: Duration::from_secs(5),
            retry_interval: Duration::from_secs(300),
            max_peers_per_round: Some(5),
            dht_retries: Some(5),
        })
    }

    pub fn announce_interval(&self) -> Duration {
        self.announce_interval
    }

    pub fn discovery_interval(&self) -> Duration {
        self.discovery_interval
    }

    pub fn discovery_interval_no_peers(&self) -> Duration {
        self.discovery_interval_no_peers
    }

    pub fn connection_timeout(&self) -> Duration {
        self.connection_timeout
    }

    pub fn retry_interval(&self) -> Duration {
        self.retry_interval
    }

    pub fn max_peers_per_round(&self) -> Option<usize> {
        self.max_peers_per_round
    }

    pub fn dht_retries(&self) -> Option<usize> {
        self.dht_retries
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
    /// Watchable neighbour count for gossip perspective
    neighbour_count: Watchable<Option<usize>>,
    /// First connection timestamp to switch discovery intervals
    first_connected_timestamp: Watchable<Option<Instant>>,
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

    async fn add_state(&self, state: &DiscoveryState) {
        let mut states = self.states.lock().await;
        states.push(state.clone());
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
                let mut c_rx = 0;
                while init_rx == c_rx || c_rx == 0 {
                    c_rx = udp_rx_bytes(paths.clone());
                    sleep(Duration::from_millis(100)).await;
                }
            })
            .await
            {
                Ok(_) => {
                    tracing::debug!(
                        "TopicDiscoveryHook: connection {} shows UDP activity, incrementing state, init={}, after={}",
                        peer_id.fmt_short(),
                        init_rx,
                        udp_rx_bytes(paths.clone())
                    );
                    let guard = states.lock().await;
                    for state in guard.iter() {
                        state.increment_connected(peer_id).await;
                    }
                }
                Err(_) => tracing::debug!(
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
        .filter_map(|path| path.stats().map(|stats| stats.udp_rx.bytes))
        .reduce(|a, b| a + b)
        .unwrap_or_default()
}

impl EndpointHooks for TopicDiscoveryHook {
    async fn after_handshake<'a>(
        &'a self,
        conn: &'a iroh::endpoint::ConnectionInfo,
    ) -> iroh::endpoint::AfterHandshakeOutcome {
        if conn.alpn() == iroh_gossip::ALPN {
            self.increment_connected(conn);
        }
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
            neighbour_count: Watchable::new(None),
            first_connected_timestamp: Watchable::new(None),
        })
    }

    fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }

    fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Relaxed)
    }

    fn has_connections(&self) -> bool {
        if let Some(neigbour_count) = self.neighbour_count.get() {
            neigbour_count > 0
        } else {
            self.connected_count.get() > 0
        }
    }

    fn connection_count(&self) -> usize {
        self.connected_count.get()
    }

    async fn increment_connected(&self, peer_id: EndpointId) {
        let mut peers = self.connected_peers.lock().await;
        if peers.insert(peer_id) {
            self.connected_count
                .set(self.connected_count.get() + 1)
                .ok();
            if self.first_connected_timestamp.get().is_none() {
                self.first_connected_timestamp
                    .set(Some(Instant::now()))
                    .ok();
            }
        }
    }

    /// Mark a peer as attempted. Returns true if we should try to connect
    /// (either new peer, or retry interval has elapsed).
    async fn should_attempt(&self, peer: [u8; 32]) -> bool {
        let mut map = self.attempted.lock().await;
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
    }

    async fn reset_attempt(&self, peer: [u8; 32]) {
        let mut map = self.attempted.lock().await;
        map.remove(&peer);
    }

    fn neighbour_count_watcher(&self) -> Watchable<Option<usize>> {
        self.neighbour_count.clone()
    }

    fn first_connected_timestamp_watcher(&self) -> Watchable<Option<Instant>> {
        self.first_connected_timestamp.clone()
    }

    fn first_connected_phase(&self, config: &TopicDiscoveryConfig) -> bool {
        if let Some(timestamp) = self.first_connected_timestamp_watcher().get()
            && let Some(first_connected_duration) = config.first_connected_duration
            && timestamp.elapsed() < first_connected_duration
        {
            true
        } else {
            false
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

    #[doc(hidden)]
    fn neighbour_count_watcher(&self) -> Watchable<Option<usize>> {
        self.state.neighbour_count_watcher()
    }

    pub async fn get_connected_peers(&self) -> Vec<EndpointId> {
        let peers = self.state.connected_peers.lock().await;
        peers.iter().cloned().collect()
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
        let neighbour_count_watcher = handle.neighbour_count_watcher();
        neighbour_count_watcher.set(Some(0)).ok();
        while let Some(event) = receiver.next().await {
            tracing::debug!("gossip_event: {:?}", event);
            if receiver.is_joined() {
                neighbour_count_watcher
                    .set(Some(receiver.neighbors().count()))
                    .ok();
                tracing::debug!("subscribe_with_discovery_joined: joined successfully");
                break;
            }
            tracing::debug!(
                "subscribe_with_discovery_joined: {:?}",
                receiver.neighbors().collect::<Vec<_>>()
            );
            sleep(Duration::from_millis(1000)).await;
        }
        neighbour_count_watcher.set(None).ok();
        //receiver.joined().await?;
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
        config.hook.add_state(&state).await;

        tracing::info!("subscribe_with_discovery: initializing shared DHT");
        let mut tries = 0;
        let dht = loop {
            if let Ok(dht) = init_dht().await {
                break Arc::new(dht);
            }
            tracing::warn!("subscribe_with_discovery: DHT init failed, retrying in 2s");
            tokio::time::sleep(Duration::from_secs(2)).await;
            tries += 1;
            if let Some(retries) = config.dht_retries()
                && tries > retries
            {
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
    match tokio::time::timeout(Duration::from_secs(15), dht.bootstrapped()).await {
        Ok(true) => {}
        Ok(false) => {
            tracing::error!("init_dht: DHT bootstrap failed");
            anyhow::bail!("DHT bootstrap failed");
        }
        Err(_) => {
            tracing::error!("init_dht: DHT bootstrap timed out");
            anyhow::bail!("DHT bootstrap timed out");
        }
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
            if state.neighbour_count_watcher().get().is_some() {
                let mut stream = state.neighbour_count.watch().stream();
                while let Some(Some(next_counter)) = stream.next().await {
                    if next_counter > 0 {
                        return true;
                    }
                }
                false
            } else {
                let mut stream = state.connected_count.watch().stream();
                while let Some(next_counter) = stream.next().await {
                    if next_counter > 0 {
                        return true;
                    }
                }
                false
            }
        };

        match time::timeout(timeout, wait_for_connection).await {
            Ok(true) => {
                tracing::info!(
                    "connector: successfully connected to peer {}",
                    peer.fmt_short()
                );
            }
            Ok(false) => {
                tracing::debug!(
                    "connector: stopped while waiting for connection to {}",
                    peer.fmt_short()
                );

                if !state.has_connections() {
                    state.reset_attempt(*peer.as_bytes()).await;
                }
            }
            Err(_) => {
                tracing::warn!(
                    "connector: timeout waiting for connection to {} after {:?}",
                    peer.fmt_short(),
                    timeout
                );
                if !state.has_connections() {
                    state.reset_attempt(*peer.as_bytes()).await;
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
            match tokio::time::timeout(
                Duration::from_secs(30),
                dht.announce_signed_peer(id, &config.signing_key),
            )
            .await
            {
                Ok(Ok(_)) => {
                    tracing::info!("announce_task: DHT announce success");
                    backoff = Duration::from_secs(5);
                    tracing::debug!("announce_task: sleeping for {:?}", config.announce_interval);
                    tokio::time::sleep(config.announce_interval).await;
                }
                Ok(Err(e)) => {
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
                Err(_) => {
                    tracing::warn!(
                        "announce_task: DHT announce timed out, retrying in {backoff:?}"
                    );
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

                if !state.should_attempt(*key_bytes).await {
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

            let has_connection = state.has_connections();
            let interval = if has_connection {
                no_peer_backoff = config.discovery_interval_no_peers;
                if state.first_connected_phase(&config) {
                    config.discovery_interval_first_connected
                } else {
                    config.discovery_interval
                }
            } else {
                let current = no_peer_backoff;
                no_peer_backoff =
                    (no_peer_backoff + backoff_increment).min(config.discovery_interval);
                current
            };

            tracing::debug!(
                "discovery_task: sleeping for {:?} (has_connections: {}, next_backoff: {:?})",
                interval,
                has_connection,
                no_peer_backoff
            );
            tokio::time::sleep(interval).await;
            if !has_connection && state.has_connections() {
                let additional_interval = if state.first_connected_phase(&config) {
                    config.discovery_interval_first_connected
                } else {
                    config.discovery_interval
                }
                .saturating_sub(interval);
                no_peer_backoff = config.discovery_interval_no_peers;
                tracing::debug!(
                    "discovery_task: conn established during sleep, additional sleep for {:?} (has_connections: {}, next_backoff: {:?})",
                    additional_interval,
                    state.has_connections(),
                    no_peer_backoff
                );
                tokio::time::sleep(additional_interval).await;
            }
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
