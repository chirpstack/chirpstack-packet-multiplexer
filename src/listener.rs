use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, anyhow};
use tokio::net::UdpSocket;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::{OnceCell, RwLock};
use tracing::{Instrument, debug, error, info, trace, warn};

use crate::monitoring::{inc_gateway_udp_received_count, inc_gateway_udp_sent_count};
use crate::packets::{GatewayId, PacketType, get_random_token};
use crate::traits::PrintFullError;

static GATEWAYS: OnceCell<RwLock<HashMap<GatewayId, Gateway>>> = OnceCell::const_new();

struct Gateway {
    addr: SocketAddr,
    last_seen: SystemTime,
}

pub async fn setup(
    bind: &str,
) -> Result<(
    UnboundedSender<(GatewayId, Vec<u8>)>,
    UnboundedReceiver<(GatewayId, Vec<u8>)>,
)> {
    info!(host = bind, "Setting up listener");

    let (uplink_tx, uplink_rx) = unbounded_channel::<(GatewayId, Vec<u8>)>();
    let (downlink_tx, downlink_rx) = unbounded_channel::<(GatewayId, Vec<u8>)>();

    let sock = UdpSocket::bind(bind).await.context("Bind socket")?;
    let sock = Arc::new(sock);

    tokio::spawn(handle_uplink(sock.clone(), uplink_tx));
    tokio::spawn(handle_downlink(sock.clone(), downlink_rx));
    tokio::spawn(cleanup_gateways());

    Ok((downlink_tx, uplink_rx))
}

async fn handle_uplink(socket: Arc<UdpSocket>, uplink_tx: UnboundedSender<(GatewayId, Vec<u8>)>) {
    let mut buffer: [u8; 65535] = [0; 65535];
    loop {
        let (size, addr) = match socket.recv_from(&mut buffer).await {
            Ok(v) => v,
            Err(e) => {
                error!(error = %e, "Receive error");
                continue;
            }
        };

        if size < 4 {
            warn!(addr = %addr, received_bytes = size, "At least 4 bytes are expected");
            continue;
        }

        if let Err(e) = handle_uplink_packet(&socket, &uplink_tx, addr, &buffer[..size])
            .instrument(tracing::info_span!("", addr = %addr))
            .await
        {
            error!(error = %e.full(), "Handle uplink packet error");
        }
    }
}

async fn handle_uplink_packet(
    socket: &Arc<UdpSocket>,
    uplink_tx: &UnboundedSender<(GatewayId, Vec<u8>)>,
    addr: SocketAddr,
    data: &[u8],
) -> Result<()> {
    let packet_type = PacketType::try_from(data)?;
    let gateway_id = GatewayId::try_from(data)?;
    let token = get_random_token(data)?;

    info!(
        packet_type = %packet_type,
        gateway_id = %gateway_id,
        token = token,
        "UDP packet received",
    );

    inc_gateway_udp_received_count(gateway_id, packet_type).await;

    match packet_type {
        PacketType::PushData => handle_push_data(socket, uplink_tx, addr, gateway_id, data).await?,
        PacketType::PullData => {
            set_gateway(gateway_id, addr).await?;
            handle_pull_data(socket, uplink_tx, addr, gateway_id, data).await?;
        }
        PacketType::TxAck => handle_tx_ack(uplink_tx, gateway_id, data).await?,
        _ => warn!(packet_type = %packet_type, "Unexpected packet-type"),
    }

    Ok(())
}

async fn handle_downlink(
    socket: Arc<UdpSocket>,
    mut downlink_rx: UnboundedReceiver<(GatewayId, Vec<u8>)>,
) {
    while let Some((gateway_id, data)) = downlink_rx.recv().await {
        if let Err(e) = handle_downlink_packet(&socket, gateway_id, &data).await {
            error!(error = %e.full(), "Handle downlink packet error");
        }
    }
}

async fn handle_downlink_packet(
    socket: &Arc<UdpSocket>,
    gateway_id: GatewayId,
    data: &[u8],
) -> Result<()> {
    let packet_type = PacketType::try_from(data)?;
    let addr = get_gateway(gateway_id).await?;
    let span = tracing::info_span!("", addr = %addr);

    async move {
        info!(packet_type = %packet_type, gateway_id = %gateway_id, "Sending UDP packet");

        socket
            .send_to(data, addr)
            .await
            .context("Socket send")
            .map(|_| ())
    }
    .instrument(span)
    .await?;

    inc_gateway_udp_sent_count(gateway_id, packet_type).await;

    Ok(())
}

async fn handle_push_data(
    socket: &Arc<UdpSocket>,
    uplink_tx: &UnboundedSender<(GatewayId, Vec<u8>)>,
    addr: SocketAddr,
    gateway_id: GatewayId,
    data: &[u8],
) -> Result<()> {
    if data.len() < 12 {
        return Err(anyhow!("At least 12 bytes are expected"));
    }

    info!(packet_type = %PacketType::PushAck, "Sending UDP packet");

    let b: [u8; 4] = [data[0], data[1], data[2], PacketType::PushAck.into()];
    socket.send_to(&b, addr).await.context("Socket send")?;
    inc_gateway_udp_sent_count(gateway_id, PacketType::PushAck).await;

    debug!("Sending received data to uplink channel");
    uplink_tx
        .send((gateway_id, data.to_vec()))
        .context("Uplink channel send")?;

    Ok(())
}

async fn handle_tx_ack(
    uplink_tx: &UnboundedSender<(GatewayId, Vec<u8>)>,
    gateway_id: GatewayId,
    data: &[u8],
) -> Result<()> {
    uplink_tx
        .send((gateway_id, data.to_vec()))
        .context("Uplink channel send")?;
    Ok(())
}

async fn handle_pull_data(
    socket: &Arc<UdpSocket>,
    uplink_tx: &UnboundedSender<(GatewayId, Vec<u8>)>,
    addr: SocketAddr,
    gateway_id: GatewayId,
    data: &[u8],
) -> Result<()> {
    if data.len() < 12 {
        return Err(anyhow!("At least 12 bytes are expected"));
    }

    info!(packet_type = %PacketType::PullAck, "Sending UDP packet");

    let b: [u8; 4] = [data[0], data[1], data[2], PacketType::PullAck.into()];
    socket.send_to(&b, addr).await.context("Socket send")?;
    inc_gateway_udp_sent_count(gateway_id, PacketType::PullAck).await;

    uplink_tx
        .send((gateway_id, data.to_vec()))
        .context("Uplink channel send")?;

    Ok(())
}

async fn set_gateway(gateway_id: GatewayId, addr: SocketAddr) -> Result<()> {
    trace!(gateway_id = %gateway_id, addr = %addr, "Setting / updating Gateway ID to addr mapping");

    let gateways = GATEWAYS
        .get_or_init(|| async { RwLock::new(HashMap::new()) })
        .await;

    let mut gateways = gateways.write().await;
    let _ = gateways.insert(
        gateway_id,
        Gateway {
            addr,
            last_seen: SystemTime::now(),
        },
    );

    Ok(())
}

async fn get_gateway(gateway_id: GatewayId) -> Result<SocketAddr> {
    trace!(gateway_id = %gateway_id, "Getting addr for Gateway ID");

    let gateways = GATEWAYS
        .get_or_init(|| async { RwLock::new(HashMap::new()) })
        .await;

    let gateways = gateways.read().await;
    gateways
        .get(&gateway_id)
        .map(|v| v.addr)
        .ok_or_else(|| anyhow!("Unknown Gateway ID: {}", gateway_id))
}

async fn cleanup_gateways() {
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;

        trace!("Cleaning up inactive Gateway ID to addr mappings");

        let gateways = GATEWAYS
            .get_or_init(|| async { RwLock::new(HashMap::new()) })
            .await;
        let mut gateways = gateways.write().await;
        gateways.retain(|k, v| {
            if let Ok(duration) = SystemTime::now().duration_since(v.last_seen) {
                if duration < Duration::from_secs(60) {
                    true
                } else {
                    warn!(gateway_id = %k, addr = %v.addr, "Cleaning up inactive mapping");
                    false
                }
            } else {
                warn!(gateway_id = %k, addr = %v.addr, "Cleaning up inactive mapping");
                false
            }
        })
    }
}
