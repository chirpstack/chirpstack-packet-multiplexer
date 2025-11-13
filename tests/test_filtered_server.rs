use std::str::FromStr;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::time::timeout;
use tracing_subscriber::prelude::*;

use chirpstack_packet_multiplexer::{config, forwarder, listener};
use lrwn_filters::EuiPrefix;

#[tokio::test]
async fn test() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .init();

    let conf = config::Configuration {
        multiplexer: config::Multiplexer {
            bind: "0.0.0.0:1710".into(),
            servers: vec![
                config::Server {
                    server: "localhost:1711".into(),
                    ..Default::default()
                },
                config::Server {
                    server: "localhost:1712".into(),
                    gateway_id_prefixes: vec![EuiPrefix::from_str("0101000000000000/16").unwrap()],
                    ..Default::default()
                },
            ],
        },
        ..Default::default()
    };

    let (downlink_tx, uplink_rx) = listener::setup(&conf.multiplexer.bind).await.unwrap();
    forwarder::setup(downlink_tx, uplink_rx, conf.multiplexer.servers.clone())
        .await
        .unwrap();
    let mut buffer: [u8; 65535] = [0; 65535];

    // Server sockets.
    let server1_sock = UdpSocket::bind("0.0.0.0:1711").await.unwrap();
    let server2_sock = UdpSocket::bind("0.0.0.0:1701").await.unwrap();

    // Gateway socket.
    let gw_sock = UdpSocket::bind("0.0.0.0:0").await.unwrap();
    gw_sock.connect("localhost:1710").await.unwrap();

    // Send PUSH_DATA.
    gw_sock
        .send(&[
            0x02, 0x01, 0x02, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x7b, 0x7d,
        ])
        .await
        .unwrap();

    // Expect PUSH_DATA forwarded to server 1.
    let size = server1_sock.recv(&mut buffer).await.unwrap();
    assert_eq!(
        &[
            0x02, 0x01, 0x02, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x7b, 0x7d,
        ],
        &buffer[..size]
    );

    // Expect PUSH_DATA not forwarded to server 2.
    let resp = timeout(Duration::from_millis(100), server2_sock.recv(&mut buffer)).await;
    assert!(resp.is_err());
}
