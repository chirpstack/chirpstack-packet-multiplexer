use std::collections::HashMap;
use std::fmt;

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug)]
pub enum PacketType {
    PushData,
    PushAck,
    PullData,
    PullResp,
    PullAck,
    TxAck,
}

impl From<PacketType> for u8 {
    fn from(p: PacketType) -> u8 {
        match p {
            PacketType::PushData => 0x00,
            PacketType::PushAck => 0x01,
            PacketType::PullData => 0x02,
            PacketType::PullResp => 0x03,
            PacketType::PullAck => 0x04,
            PacketType::TxAck => 0x05,
        }
    }
}

impl TryFrom<&[u8]> for PacketType {
    type Error = anyhow::Error;

    fn try_from(v: &[u8]) -> Result<PacketType> {
        if v.len() < 4 {
            return Err(anyhow!("At least 4 bytes are expected"));
        }

        Ok(match v[3] {
            0x00 => PacketType::PushData,
            0x01 => PacketType::PushAck,
            0x02 => PacketType::PullData,
            0x03 => PacketType::PullResp,
            0x04 => PacketType::PullAck,
            0x05 => PacketType::TxAck,
            _ => return Err(anyhow!("Invalid packet-type: {}", v[3])),
        })
    }
}

impl fmt::Display for PacketType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Clone, Copy, Debug)]
pub enum ProtocolVersion {
    Version1,
    Version2,
}

impl TryFrom<&[u8]> for ProtocolVersion {
    type Error = anyhow::Error;

    fn try_from(v: &[u8]) -> Result<ProtocolVersion> {
        if v.is_empty() {
            return Err(anyhow!("At least 1 byte is expected"));
        }

        Ok(match v[0] {
            0x01 => ProtocolVersion::Version1,
            0x02 => ProtocolVersion::Version2,
            _ => return Err(anyhow!("Unexpected protocol")),
        })
    }
}

#[derive(Clone, Copy, Eq, PartialEq, Hash, Debug)]
pub struct GatewayId([u8; 8]);

impl GatewayId {
    pub fn as_bytes_le(&self) -> [u8; 8] {
        let mut out = self.0;
        out.reverse(); // BE => LE
        out
    }
}

impl TryFrom<&[u8]> for GatewayId {
    type Error = anyhow::Error;

    fn try_from(v: &[u8]) -> Result<GatewayId> {
        if v.len() < 12 {
            return Err(anyhow!("At least 12 bytes are expected"));
        }

        let mut gateway_id: [u8; 8] = [0; 8];
        gateway_id.copy_from_slice(&v[4..12]);
        Ok(GatewayId(gateway_id))
    }
}

impl fmt::Display for GatewayId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(self.0))
    }
}

pub fn get_random_token(v: &[u8]) -> Result<u16> {
    if v.len() < 3 {
        return Err(anyhow!("At least 3 bytes are expected"));
    }

    Ok(u16::from_be_bytes([v[1], v[2]]))
}

pub struct PushData {
    pub protocol_version: u8,
    pub random_token: u16,
    pub gateway_id: [u8; 8],
    pub payload: PushDataPayload,
}

impl PushData {
    pub fn from_slice(b: &[u8]) -> Result<Self> {
        if b.len() < 14 {
            return Err(anyhow!("At least 14 bytes are expected"));
        }

        Ok(PushData {
            protocol_version: b[0],
            random_token: u16::from_be_bytes([b[1], b[2]]),
            gateway_id: {
                let mut gateway_id: [u8; 8] = [0; 8];
                gateway_id.copy_from_slice(&b[4..12]);
                gateway_id
            },
            payload: serde_json::from_slice(&b[12..])?,
        })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = vec![self.protocol_version];

        b.append(&mut self.random_token.to_be_bytes().to_vec());
        b.push(0x00);
        b.append(&mut self.gateway_id.to_vec());

        let mut j = serde_json::to_vec(&self.payload).unwrap();
        b.append(&mut j);

        b
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PushDataPayload {
    pub rxpk: Vec<RxPk>,

    // Capture all the other fields.
    #[serde(flatten)]
    other: HashMap<String, serde_json::Value>,
}

impl PushDataPayload {
    pub fn filter_rxpk(&mut self, filter: &lrwn_filters::Filters) {
        self.rxpk = self
            .rxpk
            .drain(..)
            .filter(|v| lrwn_filters::matches(&v.data, filter))
            .collect();
    }

    pub fn is_empty(&self) -> bool {
        self.rxpk.is_empty() && self.other.is_empty()
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RxPk {
    #[serde(with = "base64_codec")]
    pub data: Vec<u8>,

    // Capture all the other fields.
    #[serde(flatten)]
    other: HashMap<String, serde_json::Value>,
}

mod base64_codec {
    use base64::{Engine as _, engine::general_purpose};
    use serde::{Deserialize, Serialize};
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &Vec<u8>, s: S) -> Result<S::Ok, S::Error> {
        let base64 = general_purpose::STANDARD.encode(v);
        String::serialize(&base64, s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let base64 = String::deserialize(d)?;
        general_purpose::STANDARD
            .decode(base64.as_bytes())
            .map_err(serde::de::Error::custom)
    }
}
