use std::{collections::HashMap, future::Future, pin::Pin, str::FromStr, sync::Arc};

use anyhow::{bail, Result};
use iroh::{
    endpoint::{Endpoint, Connecting, RecvStream, SendStream},
    protocol::ProtocolHandler,
    NodeAddr, NodeId, SecretKey,
};
use iroh_gossip::proto::TopicId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::utils::wait_for_relay;

#[derive(Debug, Clone)]
pub struct TopicTracker {
    pub node_id: NodeId,
    endpoint: Endpoint,
    kv: Arc<Mutex<HashMap<[u8; 32], Vec<NodeId>>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum Protocol {
    TopicRequest((Topic, NodeId)),
    TopicList(Vec<NodeId>),
    Done,
}

impl TopicTracker {
    pub const ALPN: &'static [u8] = b"iroh/topictracker/1";
    pub const MAX_TOPIC_LIST_SIZE: usize = 10;
    pub const MAX_NODE_IDS_PER_TOPIC: usize = 100;
    pub const BOOTSTRAP_NODES: &str =
        "abcdef4df4d74587095d071406c2a8462bde5079cbbc0c50051b9b2e84d67691";

    pub fn new(endpoint: &Endpoint) -> Self {
        let me = Self {
            endpoint: endpoint.clone(),
            node_id: endpoint.node_id(),
            kv: Arc::new(Mutex::new(HashMap::new())),
        };

        tokio::spawn({
            let me2 = me.clone();
            async move {
                while let Some(connecting) = me2.clone().endpoint.accept().await {
                    match connecting.accept() {
                        Ok(conn) => {
                            tokio::spawn({
                                let me3 = me2.clone();
                                async move {
                                    let _ = me3.accept(conn).await;
                                }
                            });
                        }
                        Err(err) => {
                            println!("Failed to connect {err}");
                        }
                    }
                }
            }
        });

        me
    }

    async fn send_msg(msg: Protocol, send: &mut SendStream) -> Result<()> {
        let encoded = postcard::to_stdvec(&msg)?;
        send.write_all(&(encoded.len() as u64).to_le_bytes())
            .await?;
        send.write_all(&encoded).await?;
        Ok(())
    }

    async fn recv_msg(recv: &mut RecvStream) -> Result<Protocol> {
        let mut incoming_len = [0u8; 8];
        recv.read_exact(&mut incoming_len).await?;
        let len = u64::from_le_bytes(incoming_len);

        let mut buffer = vec![0u8; len as usize];
        recv.read_exact(&mut buffer).await?;
        let msg: Protocol = postcard::from_bytes(&buffer)?;
        Ok(msg)
    }

    pub async fn get_topic_nodes(self: Arc<Self>, topic: &Topic) -> Result<Vec<NodeId>> {
        wait_for_relay(&self.endpoint).await?;

        let conn = self
            .endpoint
            .connect(
                NodeAddr::new(NodeId::from_str(Self::BOOTSTRAP_NODES)?),
                Self::ALPN,
            )
            .await?;

        let (mut send, mut recv) = conn.open_bi().await?;

        let msg = Protocol::TopicRequest((topic.clone(), self.node_id.clone()));
        Self::send_msg(msg, &mut send).await?;

        let back = match Self::recv_msg(&mut recv).await? {
            Protocol::TopicList(vec) => {
                let mut _kv = self.kv.lock().await;
                match _kv.get_mut(&topic.0) {
                    Some(node_ids) => {
                        for id in vec.clone() {
                            if node_ids.contains(&id) {
                                node_ids.retain(|nid| !nid.eq(&id));
                            }
                            node_ids.push(id);
                        }
                    }
                    None => {
                        let mut node_ids = Vec::with_capacity(Self::MAX_NODE_IDS_PER_TOPIC);
                        for id in vec.clone() {
                            node_ids.push(id);
                        }
                        _kv.insert(topic.0, node_ids);
                    }
                };
                drop(_kv);
                Ok(vec)
            }
            _ => bail!("illegal message received"),
        };

        Self::send_msg(Protocol::Done, &mut send).await?;
        send.finish()?;
        back
    }

    async fn accept(&self, conn: Connecting) -> Result<()> {

        let (mut send, mut recv) = conn.await?.accept_bi().await?;
        let msg = Self::recv_msg(&mut recv).await?;

        match msg {
            Protocol::TopicRequest((topic, remote_node_id)) => {
                let mut _kv = self.kv.lock().await;
                let resp;
                match _kv.get_mut(&topic.0) {
                    Some(node_ids) => {
                        let latest_list = node_ids
                            .iter()
                            .filter(|&i| !i.eq(&remote_node_id))
                            .rev()
                            .take(Self::MAX_TOPIC_LIST_SIZE)
                            .map(|i| *i)
                            .collect();

                        resp = Protocol::TopicList(latest_list);

                        if node_ids.contains(&remote_node_id) {
                            node_ids.retain(|nid| !nid.eq(&remote_node_id));
                        }
                        node_ids.push(remote_node_id);
                    }
                    None => {
                        let mut node_ids = Vec::with_capacity(Self::MAX_NODE_IDS_PER_TOPIC);
                        node_ids.push(remote_node_id);
                        _kv.insert(topic.0, node_ids);
                        
                        resp = Protocol::TopicList(vec![]);
                    }
                };

                Self::send_msg(resp, &mut send).await?;
                Self::send_msg(Protocol::Done,&mut send).await?;
                Self::recv_msg(&mut recv).await?;

                drop(_kv);
            }
            _ => {
                bail!("Illegal request");
            }
        };

        send.finish()?;
        Ok(())
    }

    pub async fn memory_footprint(&self) -> usize {
        let _kv = self.kv.lock().await;
        let val = &*_kv;
        size_of_val(val)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Topic([u8; 32]);

impl Topic {
    pub fn new(topic: [u8; 32]) -> Self {
        Self(topic)
    }

    pub fn from_passphrase(phrase: &str) -> Self {
        Self(Self::hash(phrase))
    }

    fn hash(s: &str) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(s);
        let mut buf = [0u8; 32];
        buf.copy_from_slice(&hasher.finalize()[..32]);
        buf
    }

    pub fn to_string(&self) -> String {
        z32::encode(&self.0)
    }

    pub fn to_secret_key(&self) -> SecretKey {
        SecretKey::from_bytes(&self.0.clone())
    }
}

impl Default for Topic {
    fn default() -> Self {
        Self::from_passphrase("password")
    }
}

impl ProtocolHandler for TopicTracker {
    fn accept(
        &self,
        conn: Connecting,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>> {
        let topic_tracker = self.clone();

        Box::pin(async move {
            topic_tracker.accept(conn).await?;

            Ok(())
        })
    }
}

#[cfg(feature="iroh-gossip-cast")]
impl Into<iroh_gossip::proto::TopicId> for Topic {
    fn into(self) -> iroh_gossip::proto::TopicId {
        TopicId::from_bytes(self.0)
    }
}

#[cfg(feature="iroh-gossip-cast")]
impl From<iroh_gossip::proto::TopicId> for Topic {
    fn from(value: iroh_gossip::proto::TopicId) -> Self {
        Self { 0: *value.as_bytes() }
    }
}