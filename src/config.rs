use std::{env, fs};

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Configuration {
    pub logging: Logging,
    pub multiplexer: Multiplexer,
    pub monitoring: Monitoring,
}

impl Configuration {
    pub fn get(filenames: &[String]) -> Result<Configuration> {
        let mut content = String::new();

        for file_name in filenames {
            content.push_str(&fs::read_to_string(file_name)?);
        }

        // Replace environment variables in config.
        for (k, v) in env::vars() {
            content = content.replace(&format!("${}", k), &v);
        }

        let config: Configuration = toml::from_str(&content)?;
        Ok(config)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Logging {
    pub level: String,
}

impl Default for Logging {
    fn default() -> Self {
        Logging {
            level: "info".into(),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Multiplexer {
    pub bind: String,
    #[serde(rename = "server")]
    pub servers: Vec<Server>,
}

impl Default for Multiplexer {
    fn default() -> Self {
        Multiplexer {
            bind: "0.0.0.0:1700".into(),
            servers: Vec::new(),
        }
    }
}

#[derive(Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Server {
    pub server: String,
    pub uplink_only: bool,
    pub gateway_id_prefixes: Vec<lrwn_filters::EuiPrefix>,
    pub filters: Filters,
}

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
pub struct Filters {
    pub dev_addr_prefixes: Vec<lrwn_filters::DevAddrPrefix>,
    pub join_eui_prefixes: Vec<lrwn_filters::EuiPrefix>,
}

#[derive(Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Monitoring {
    pub bind: String,
}
