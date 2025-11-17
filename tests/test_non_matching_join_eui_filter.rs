use std::str::FromStr;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::time::timeout;
use tracing_subscriber::prelude::*;

use chirpstack_packet_multiplexer::{config, forwarder, listener};

#[tokio::test]
async fn test() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .init();

    let conf = config::Configuration {
        multiplexer: config::Multiplexer {
            bind: "0.0.0.0:1710".into(),
            servers: vec![config::Server {
                server: "localhost:1711".into(),
                filters: config::Filters {
                    join_eui_prefixes: vec![
                        lrwn_filters::EuiPrefix::from_str("0200000000000000/8").unwrap(),
                    ],
                    ..Default::default()
                },
                ..Default::default()
            }],
        },
        ..Default::default()
    };

    let (downlink_tx, uplink_rx) = listener::setup(&conf.multiplexer.bind).await.unwrap();
    forwarder::setup(downlink_tx, uplink_rx, conf.multiplexer.servers.clone())
        .await
        .unwrap();
    let mut buffer: [u8; 65535] = [0; 65535];

    // Server socket.
    let server_sock = UdpSocket::bind("0.0.0.0:1711").await.unwrap();

    // Gateway socket.
    let gw_sock = UdpSocket::bind("0.0.0.0:0").await.unwrap();
    gw_sock.connect("localhost:1710").await.unwrap();

    // Send PUSH_DATA (join-request with JoinEUI 0102030405060708).
    gw_sock
        .send(&[
            0x02, 0x01, 0x02, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x7b, 0x22,
            0x72, 0x78, 0x70, 0x6b, 0x22, 0x3a, 0x5b, 0x7b, 0x22, 0x64, 0x61, 0x74, 0x61, 0x22,
            0x3a, 0x22, 0x41, 0x41, 0x67, 0x48, 0x42, 0x67, 0x55, 0x45, 0x41, 0x77, 0x49, 0x42,
            0x41, 0x41, 0x41, 0x41, 0x41, 0x41, 0x41, 0x41, 0x41, 0x41, 0x41, 0x3d, 0x22, 0x7d,
            0x5d, 0x7d,
        ])
        .await
        .unwrap();

    // Expect PUSH_ACK.
    let size = gw_sock.recv(&mut buffer).await.unwrap();
    assert_eq!(&[0x02, 0x01, 0x02, 0x01], &buffer[..size]);

    // Expect PUSH_DATA not to be forwarded.
    let resp = timeout(Duration::from_millis(100), server_sock.recv(&mut buffer)).await;
    assert!(resp.is_err());
}
