use std::str::FromStr;

use clap::{Parser, Subcommand};
use signal_hook::{consts::SIGINT, consts::SIGTERM, iterator::Signals};
use tracing::{Level, info};
use tracing_subscriber::{filter, prelude::*};

use chirpstack_packet_multiplexer::{cmd, config, forwarder, listener, monitoring};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[arg(short, long, value_name = "FILE")]
    config: Vec<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Print the configuration template
    Configfile {},
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let config = config::Configuration::get(&cli.config).expect("Read configuration");

    if let Some(Commands::Configfile {}) = &cli.command {
        cmd::configfile::run(&config);
        return;
    }

    let filter = filter::Targets::new().with_targets(vec![(
        "chirpstack_packet_multiplexer",
        Level::from_str(&config.logging.level).unwrap(),
    )]);

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(filter)
        .init();

    info!(
        "Starting {} (version: {}, docs: {})",
        env!("CARGO_PKG_DESCRIPTION"),
        env!("CARGO_PKG_VERSION"),
        env!("CARGO_PKG_HOMEPAGE"),
    );

    let (downlink_tx, uplink_rx) = listener::setup(&config.multiplexer.bind)
        .await
        .expect("Setup listener");
    forwarder::setup(downlink_tx, uplink_rx, config.multiplexer.servers.clone())
        .await
        .expect("Setup forwarder");
    monitoring::setup(&config.monitoring.bind)
        .await
        .expect("Setup monitoring");

    let mut signals = Signals::new([SIGINT, SIGTERM]).unwrap();
    signals.forever().next();
}
