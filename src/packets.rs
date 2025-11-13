use std::fmt;

use anyhow::{Result, anyhow};

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
