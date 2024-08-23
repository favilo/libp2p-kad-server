use std::{net::Ipv4Addr, time::Duration};

use futures::StreamExt;
use libp2p::{
    core::muxing::StreamMuxerBox,
    dcutr, identify,
    identity::Keypair,
    kad::{self, store::MemoryStore},
    multiaddr::Protocol,
    noise, ping, relay,
    swarm::NetworkBehaviour,
    tcp, yamux, Multiaddr, PeerId, SwarmBuilder, Transport as _,
};
use rand::thread_rng;
use tokio::io::AsyncWriteExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("libp2p_kad_server=debug,info")
        .try_init();

    let keypair = load_or_generate_keypair().await?;
    tracing::info!("Loaded keypair: {}", PeerId::from(keypair.public()));

    let mut swarm = SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_quic()
        .with_other_transport(|id_keys| {
            let transport = libp2p_webrtc::tokio::Transport::new(
                id_keys.clone(),
                libp2p_webrtc::tokio::Certificate::generate(&mut thread_rng())?,
            );
            Ok(transport.map(|(peer_id, conn), _| (peer_id, StreamMuxerBox::new(conn))))
        })?
        .with_dns()?
        .with_relay_client(noise::Config::new, yamux::Config::default)?
        .with_behaviour(Behaviour::from_key)?
        .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(60)))
        .build();

    let addr_webrtc = Multiaddr::from(Ipv4Addr::UNSPECIFIED).with(Protocol::Udp(30333));
    let addr_tcp = Multiaddr::from(Ipv4Addr::UNSPECIFIED).with(Protocol::Tcp(30333));

    swarm.listen_on(addr_webrtc)?;
    swarm.listen_on(addr_tcp)?;

    loop {
        tokio::select! {
            swarm_event = swarm.next() => {
                tracing::info!(?swarm_event);
            }
            _ = tokio::signal::ctrl_c() => {
                break;
            }
        }
    }

    Ok(())
}

async fn load_or_generate_keypair() -> anyhow::Result<Keypair> {
    // Fetch the keypair from the /app/config/keypair.json file
    // If it doesn't exist, generate a new keypair and save it to the file

    let file_path = "/app/config/keypair.json";
    if tokio::fs::metadata(file_path).await.is_err() {
        let keypair = Keypair::generate_ed25519();
        let mut file = tokio::fs::File::create(file_path).await?;
        file.write_all(keypair.to_protobuf_encoding()?.as_ref())
            .await?;
        return Ok(keypair);
    }
    let bytes = tokio::fs::read(file_path).await?;
    let keypair: Keypair = Keypair::from_protobuf_encoding(&bytes)?;
    Ok(keypair)
}

#[derive(NetworkBehaviour)]
struct Behaviour {
    pub kad: kad::Behaviour<MemoryStore>,
    pub ping: ping::Behaviour,
    pub identify: identify::Behaviour,
    pub relay_server: relay::Behaviour,
    pub relay_client: relay::client::Behaviour,
    pub dcutr: dcutr::Behaviour,
}

impl Behaviour {
    pub fn from_key(
        keypair: &Keypair,
        relay_client: relay::client::Behaviour,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let peer_id = PeerId::from_public_key(&keypair.public());
        let kad = kad::Behaviour::new(peer_id, MemoryStore::new(peer_id));
        let ping = ping::Behaviour::new(ping::Config::new());
        let identify = identify::Behaviour::new(identify::Config::new(
            "/ip4/0.0.0.0/udp/0".parse().unwrap(),
            keypair.public(),
        ));
        let relay_server = relay::Behaviour::new(peer_id, relay::Config::default());
        let dcutr = dcutr::Behaviour::new(peer_id);

        Ok(Self {
            kad,
            ping,
            identify,
            relay_server,
            relay_client,
            dcutr,
        })
    }
}
