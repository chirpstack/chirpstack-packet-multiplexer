use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, anyhow};
use tokio::net::UdpSocket;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::{OnceCell, RwLock, oneshot};
use tracing::{Instrument, debug, error, info, trace, warn};

use crate::config;
use crate::monitoring::{inc_server_udp_received_count, inc_server_udp_sent_count};
use crate::packets::{GatewayId, PacketType, PushData, get_random_token};
use crate::traits::PrintFullError;

static SERVERS: OnceCell<RwLock<Vec<Server>>> = OnceCell::const_new();

struct Server {
    server: String,
    uplink_only: bool,
    gateway_id_prefixes: Vec<lrwn_filters::EuiPrefix>,
    filters: lrwn_filters::Filters,
    downlink_tx: UnboundedSender<(GatewayId, Vec<u8>)>,
    sockets: HashMap<GatewayId, ServerSocket>,
}

impl Server {
    fn match_prefixes(&self, gateway_id: GatewayId) -> bool {
        let gw_id_le = gateway_id.as_bytes_le();
        if self.gateway_id_prefixes.is_empty() {
            return true;
        }

        for prefix in &self.gateway_id_prefixes {
            if prefix.is_match(gw_id_le) {
                return true;
            }
        }

        false
    }

    async fn get_server_socket(&mut self, gateway_id: GatewayId) -> Result<&mut ServerSocket> {
        // Check if we already have a socket for the given Gateway ID to the
        // server and if not, we create it.
        if let std::collections::hash_map::Entry::Vacant(e) = self.sockets.entry(gateway_id) {
            info!(gateway_id = %gateway_id, server = %self.server, "Initializing forwarder to server");

            let socket = UdpSocket::bind("0.0.0.0:0")
                .await
                .context("UDP socket bind")?;
            socket
                .connect(&self.server)
                .await
                .context("UDP socket connect")?;

            let socket = Arc::new(socket);
            let (stop_tx, stop_rx) = oneshot::channel::<()>();

            tokio::spawn(handle_downlink(
                self.server.clone(),
                stop_rx,
                self.uplink_only,
                socket.clone(),
                self.downlink_tx.clone(),
                gateway_id,
            ));

            e.insert(ServerSocket {
                last_uplink: SystemTime::now(),
                push_data_token: None,
                pull_data_token: None,
                pull_resp_token: None,
                _stop_tx: stop_tx,
                socket,
            });
        }

        // This should never error since we check the existence of the GatewayId key above.
        let socket = self
            .sockets
            .get_mut(&gateway_id)
            .ok_or_else(|| anyhow!("Gateway ID not found"))?;

        Ok(socket)
    }
}

struct ServerSocket {
    last_uplink: SystemTime,
    _stop_tx: oneshot::Sender<()>,
    socket: Arc<UdpSocket>,
    pull_data_token: Option<u16>,
    push_data_token: Option<u16>,
    pull_resp_token: Option<u16>,
}

pub async fn setup(
    downlink_tx: UnboundedSender<(GatewayId, Vec<u8>)>,
    uplink_rx: UnboundedReceiver<(GatewayId, Vec<u8>)>,
    servers: Vec<config::Server>,
) -> Result<()> {
    info!("Setting up forwarder");

    for server in servers {
        add_server(
            server.server.clone(),
            server.uplink_only,
            server.gateway_id_prefixes.clone(),
            lrwn_filters::Filters {
                dev_addr_prefixes: server.filters.dev_addr_prefixes.clone(),
                join_eui_prefixes: server.filters.join_eui_prefixes.clone(),
            },
            downlink_tx.clone(),
        )
        .await?;
    }

    tokio::spawn(handle_uplink(uplink_rx));
    tokio::spawn(cleanup_sockets());

    Ok(())
}

async fn handle_uplink(mut uplink_rx: UnboundedReceiver<(GatewayId, Vec<u8>)>) {
    while let Some((gateway_id, data)) = uplink_rx.recv().await {
        if let Err(e) = handle_uplink_packet(gateway_id, &data).await {
            error!(error = %e.full(), "Handle uplink error");
        }
    }
}

async fn handle_uplink_packet(gateway_id: GatewayId, data: &[u8]) -> Result<()> {
    let packet_type = PacketType::try_from(data)?;
    let random_token = get_random_token(data)?;

    let servers = SERVERS
        .get_or_init(|| async { RwLock::new(Vec::new()) })
        .await;
    let mut servers = servers.write().await;

    for server in servers.iter_mut() {
        if !server.match_prefixes(gateway_id) {
            continue;
        }

        let filters = server.filters.clone();
        let socket = server.get_server_socket(gateway_id).await?;
        socket.last_uplink = SystemTime::now();

        let span = tracing::info_span!("", addr = %socket.socket.peer_addr().unwrap());
        let _enter = span.enter();

        match packet_type {
            PacketType::PushData => match PushData::from_slice(data) {
                Ok(mut push_data) => {
                    push_data.payload.filter_rxpk(&filters);

                    if push_data.payload.is_empty() {
                        debug!("Nothing to send, UDP packet is empty or does not match filters");
                    } else {
                        info!(packet_type = %packet_type, "Sending UDP packet");
                        socket.push_data_token = Some(random_token);
                        socket.socket.send(data).await.context("Send UDP packet")?;
                        inc_server_udp_sent_count(&server.server, packet_type).await;
                    }
                }
                Err(e) => {
                    error!("Decode PushData payload error: {}", e);
                }
            },
            PacketType::PullData => {
                info!(packet_type = %packet_type, "Sending UDP packet");
                socket.pull_data_token = Some(random_token);
                socket.socket.send(data).await.context("Send UDP packet")?;
                inc_server_udp_sent_count(&server.server, packet_type).await;
            }
            PacketType::TxAck => {
                if let Some(pull_resp_token) = socket.pull_resp_token
                    && pull_resp_token == random_token
                {
                    info!(packet_type = %packet_type, "Sending UDP packet");
                    socket.pull_resp_token = None;
                    socket.socket.send(data).await.context("Send UDP packet")?;
                    inc_server_udp_sent_count(&server.server, packet_type).await;
                }
            }
            _ => {}
        }
    }

    Ok(())
}

async fn handle_downlink(
    server: String,
    mut stop_rx: oneshot::Receiver<()>,
    uplink_only: bool,
    socket: Arc<UdpSocket>,
    downlink_tx: UnboundedSender<(GatewayId, Vec<u8>)>,
    gateway_id: GatewayId,
) {
    let mut buffer: [u8; 65535] = [0; 65535];

    loop {
        let (size, addr) = tokio::select! {
            _ = &mut stop_rx => {
                break;
            }
           v = socket.recv_from(&mut buffer) =>
                match v  {
                    Ok(v) => v,
                    Err(e) => {
                        error!(error = %e, "UDP socket receive error");
                        break;
                    },
                },
            else => {
                break;
            }
        };

        if size < 4 {
            warn!(addr = %addr, received_bytes = size, "At least 4 bytes are expected");
            continue;
        }

        if let Err(e) = handle_downlink_packet(
            &server,
            uplink_only,
            &downlink_tx,
            gateway_id,
            &buffer[..size],
        )
        .instrument(tracing::info_span!("", addr = %addr, gateway_id = %gateway_id))
        .await
        {
            error!(error = %e.full(), "Handle downlink packet error");
        }
    }

    debug!("Downlink loop has ended");
}

async fn handle_downlink_packet(
    server: &str,
    uplink_only: bool,
    downlink_tx: &UnboundedSender<(GatewayId, Vec<u8>)>,
    gateway_id: GatewayId,
    data: &[u8],
) -> Result<()> {
    let packet_type = PacketType::try_from(data)?;
    let token = get_random_token(data)?;

    info!(packet_type = %packet_type, token = token, "UDP packet received");

    inc_server_udp_received_count(server, packet_type).await;

    match packet_type {
        PacketType::PullResp => {
            if uplink_only {
                warn!("Dropping downlink, server is configured as uplink-only");
            } else {
                set_pull_resp_token(server, gateway_id, token).await?;
                handle_pull_resp(downlink_tx, gateway_id, data).await?;
            }
        }
        PacketType::PullAck => {
            // TODO: keep ack stats
        }
        PacketType::PushAck => {
            // TODO: keep ack stats
        }

        _ => {}
    }

    Ok(())
}

async fn handle_pull_resp(
    downlink_tx: &UnboundedSender<(GatewayId, Vec<u8>)>,
    gateway_id: GatewayId,
    data: &[u8],
) -> Result<()> {
    debug!("Sending received data to downlink channel");
    downlink_tx
        .send((gateway_id, data.to_vec()))
        .context("Downlink channel send")?;

    Ok(())
}

async fn add_server(
    server: String,
    uplink_only: bool,
    gateway_id_prefixes: Vec<lrwn_filters::EuiPrefix>,
    filters: lrwn_filters::Filters,
    downlink_tx: UnboundedSender<(GatewayId, Vec<u8>)>,
) -> Result<()> {
    info!(
        server = server,
        uplink_only = uplink_only,
        gateway_id_prefixes = ?gateway_id_prefixes,
        "Adding server"
    );

    let servers = SERVERS
        .get_or_init(|| async { RwLock::new(Vec::new()) })
        .await;

    let mut servers = servers.write().await;
    servers.push(Server {
        server,
        uplink_only,
        gateway_id_prefixes,
        filters,
        downlink_tx,
        sockets: HashMap::new(),
    });

    Ok(())
}

async fn cleanup_sockets() {
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;

        trace!("Cleaning up inactive sockets");

        let servers = SERVERS
            .get_or_init(|| async { RwLock::new(Vec::new()) })
            .await;
        let mut servers = servers.write().await;

        for server in servers.iter_mut() {
            server.sockets.retain(|k, v| {
                if let Ok(duration) = SystemTime::now().duration_since(v.last_uplink) {
                    if duration < Duration::from_secs(60) {
                        true
                    } else {
                        warn!(server = server.server, gateway_id = %k, "Cleaning up inactive socket");
                        false
                    }
                } else {
                    warn!(server = server.server, gateway_id = %k, "Cleaning up inactive socket");
                    false
                }
            });
        }
    }
}

async fn set_pull_resp_token(srv: &str, gateway_id: GatewayId, token: u16) -> Result<()> {
    let servers = SERVERS
        .get_or_init(|| async { RwLock::new(Vec::new()) })
        .await;
    let mut servers = servers.write().await;

    for server in servers.iter_mut() {
        if server.server.eq(srv)
            && let Some(v) = server.sockets.get_mut(&gateway_id)
        {
            v.pull_resp_token = Some(token);
        }
    }

    Ok(())
}
