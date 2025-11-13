use anyhow::Result;
use axum::{Router, http::StatusCode, routing::get};
use prometheus_client::{
    encoding::EncodeLabelSet,
    encoding::text::encode,
    metrics::counter::Counter,
    metrics::family::Family,
    registry::{Metric, Registry},
};
use tokio::net::TcpListener;
use tokio::sync::{OnceCell, RwLock};
use tracing::info;

use crate::packets::{GatewayId, PacketType};

static REGISTRY: OnceCell<RwLock<Registry>> = OnceCell::const_new();
static GATEWAY_UDP_SENT_COUNT: OnceCell<Family<GatewayUdpLabels, Counter>> = OnceCell::const_new();
static GATEWAY_UDP_RECEIVED_COUNT: OnceCell<Family<GatewayUdpLabels, Counter>> =
    OnceCell::const_new();
static SERVER_UDP_SENT_COUNT: OnceCell<Family<ServerUdpLabels, Counter>> = OnceCell::const_new();
static SERVER_UDP_RECEIVED_COUNT: OnceCell<Family<ServerUdpLabels, Counter>> =
    OnceCell::const_new();

#[derive(Clone, Hash, PartialEq, Eq, EncodeLabelSet, Debug)]
struct GatewayUdpLabels {
    gateway_id: String,
    r#type: String,
}

#[derive(Clone, Hash, PartialEq, Eq, EncodeLabelSet, Debug)]
struct ServerUdpLabels {
    server: String,
    r#type: String,
}

pub async fn setup(bind: &str) -> Result<()> {
    if bind.is_empty() {
        info!("Monitoring endpoint is not configured");
        return Ok(());
    }

    info!(bind = bind, "Setting up monitoring endpoint");

    let app = Router::new().route("/metrics", get(get_prometheus_metrics));

    let listener = TcpListener::bind(bind).await?;
    tokio::spawn(async {
        axum::serve(listener, app).await.unwrap();
    });

    Ok(())
}

async fn get_prometheus_metrics() -> (StatusCode, String) {
    let registry = REGISTRY
        .get_or_init(|| async { RwLock::new(Registry::default()) })
        .await;

    let registry = registry.read().await;
    let mut buffer = String::new();
    if let Err(e) = encode(&mut buffer, &registry) {
        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    } else {
        (StatusCode::OK, buffer)
    }
}

pub async fn register(name: &str, help: &str, metric: impl Metric) {
    let registry = REGISTRY
        .get_or_init(|| async { RwLock::new(Registry::default()) })
        .await;
    let mut registry = registry.write().await;
    registry.register(name, help, metric)
}

pub async fn inc_gateway_udp_sent_count(gateway_id: GatewayId, packet_type: PacketType) {
    let counter = GATEWAY_UDP_SENT_COUNT
        .get_or_init(|| async {
            let counter = Family::<GatewayUdpLabels, Counter>::default();
            register(
                "gateway_udp_sent_count",
                "Number of UDP datagrams sent to the gateway",
                counter.clone(),
            )
            .await;
            counter
        })
        .await;

    counter
        .get_or_create(&GatewayUdpLabels {
            gateway_id: gateway_id.to_string(),
            r#type: packet_type.to_string(),
        })
        .inc();
}

pub async fn inc_gateway_udp_received_count(gateway_id: GatewayId, packet_type: PacketType) {
    let counter = GATEWAY_UDP_RECEIVED_COUNT
        .get_or_init(|| async {
            let counter = Family::<GatewayUdpLabels, Counter>::default();
            register(
                "gateway_udp_received_count",
                "Number of UDP datagrams received from the gateway",
                counter.clone(),
            )
            .await;
            counter
        })
        .await;

    counter
        .get_or_create(&GatewayUdpLabels {
            gateway_id: gateway_id.to_string(),
            r#type: packet_type.to_string(),
        })
        .inc();
}

pub async fn inc_server_udp_sent_count(server: &str, packet_type: PacketType) {
    let counter = SERVER_UDP_SENT_COUNT
        .get_or_init(|| async {
            let counter = Family::<ServerUdpLabels, Counter>::default();
            register(
                "server_udp_sent_count",
                "Number of UDP datagrams sent to the server",
                counter.clone(),
            )
            .await;
            counter
        })
        .await;

    counter
        .get_or_create(&ServerUdpLabels {
            server: server.to_string(),
            r#type: packet_type.to_string(),
        })
        .inc();
}

pub async fn inc_server_udp_received_count(server: &str, packet_type: PacketType) {
    let counter = SERVER_UDP_RECEIVED_COUNT
        .get_or_init(|| async {
            let counter = Family::<ServerUdpLabels, Counter>::default();
            register(
                "server_udp_received_count",
                "Number of UDP datagrams received from the server",
                counter.clone(),
            )
            .await;
            counter
        })
        .await;

    counter
        .get_or_create(&ServerUdpLabels {
            server: server.to_string(),
            r#type: packet_type.to_string(),
        })
        .inc();
}
