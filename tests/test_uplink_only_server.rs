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
                uplink_only: true,
                server: "localhost:1711".into(),
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

    // Send PUSH_DATA.
    gw_sock
        .send(&[
            0x02, 0x01, 0x02, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x7b, 0x7d,
        ])
        .await
        .unwrap();

    // Expect PUSH_ACK.
    let size = gw_sock.recv(&mut buffer).await.unwrap();
    assert_eq!(&[0x02, 0x01, 0x02, 0x01], &buffer[..size]);

    // Expect PUSH_DATA forwarded to server.
    let size = server_sock.recv(&mut buffer).await.unwrap();
    assert_eq!(
        &[
            0x02, 0x01, 0x02, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x7b, 0x7d,
        ],
        &buffer[..size]
    );

    // Send PULL_DATA.
    gw_sock
        .send(&[
            0x02, 0x01, 0x02, 0x02, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
        ])
        .await
        .unwrap();

    // Expect PULL_ACK.
    let size = gw_sock.recv(&mut buffer).await.unwrap();
    assert_eq!(&[0x02, 0x01, 0x02, 0x04], &buffer[..size]);

    // Expect PULL_ACK forwarded to server.
    let (size, addr) = server_sock.recv_from(&mut buffer).await.unwrap();
    assert_eq!(
        &[
            0x02, 0x01, 0x02, 0x02, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
        ],
        &buffer[..size]
    );

    // Send PULL_RESP from server.
    server_sock
        .send_to(&[0x02, 0x01, 0x02, 0x03, 0x7b, 0x7d], addr)
        .await
        .unwrap();

    // Expect PULL_RESP not forwarded to gateway.
    let resp = timeout(Duration::from_millis(100), gw_sock.recv(&mut buffer)).await;
    assert!(resp.is_err());
}
