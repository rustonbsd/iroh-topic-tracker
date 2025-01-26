use std::net::{Ipv4Addr, SocketAddrV4};

use anyhow::bail;
use futures_lite::StreamExt;
use iroh::{Endpoint, PublicKey, SecretKey};
use iroh_gossip::net::{Event, Gossip, GossipEvent};
use rand::{rngs::OsRng, Rng};

use iroh_topic_tracker::integrations::iroh_gossip::*;
use iroh_topic_tracker::topic_tracker::Topic;

#[tokio::test]
async fn e2e_gossip_autodiscovery_test() -> anyhow::Result<()> {
    // Test topic creation and conversion
    let topic = Topic::new(OsRng.gen::<[u8; 32]>());

    // Create endpoint0 with nodeid0 and topic_tracker0
    let addr0 = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0);
    let endpoint0 = Endpoint::builder()
        .discovery_n0()
        .bind_addr_v4(addr0)
        .secret_key(SecretKey::generate(rand::rngs::OsRng))
        .bind()
        .await?;

    let mut gossip0 = Gossip::builder()
        .spawn_with_auto_discovery(endpoint0.clone())
        .await?;
    let _router0 = iroh::protocol::Router::builder(endpoint0.clone())
        .accept(iroh_gossip::ALPN, gossip0.gossip.clone())
        .spawn()
        .await?;


    // Create endpoint1 with nodeid1 and topic_tracker1
    let addr1 = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0);
    let endpoint1 = Endpoint::builder()
        .discovery_n0()
        .secret_key(SecretKey::generate(rand::rngs::OsRng))
        .bind_addr_v4(addr1)
        .bind()
        .await?;

    let mut gossip1 = Gossip::builder()
        .spawn_with_auto_discovery(endpoint1.clone())
        .await?;
    let _router1 = iroh::protocol::Router::builder(endpoint1.clone())
        .accept(iroh_gossip::ALPN, gossip1.gossip.clone())
        .spawn()
        .await?;

    

    // Await messages
    if let (Ok(res0), Ok(res1)) = tokio::join!(await_message(&mut gossip0, &topic, endpoint1.node_id()),
    await_message(&mut gossip1, &topic, endpoint0.node_id())) {
        assert!(res0);
        assert!(res1);
        Ok(())
    } else {
        bail!("failed e2e iroh-gossip test")
    }
    
}

async fn await_message(
    gossip: &mut GossipAutoDiscovery,
    topic: &Topic,
    expected_node_id: PublicKey,
) -> anyhow::Result<bool> {

    let mut gossip_client = gossip.subscribe_and_join(topic.clone().into()).await?;

    // Send messages
    gossip_client.broadcast(format!("hello iroh {}",expected_node_id).into()).await?;

    let mut required_events_1_count = 0;
    while let Some(event) = gossip_client.next().await {
        match event {
            Ok(Event::Gossip(GossipEvent::Received(msg))) => {
                assert!(String::from_utf8(msg.content.to_vec())?.starts_with("hello iroh "));
                required_events_1_count += 1;
                return Ok(required_events_1_count == 1);
            }
            _ => {
                
            }
        }
    }
    Ok(true)
}
