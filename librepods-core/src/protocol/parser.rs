// LibrePods - AirPods liberated from Apple's ecosystem
// Copyright (C) 2025 LibrePods contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Incoming AAP packet parser.
//!
//! Dispatches raw bytes received from the L2CAP socket into typed
//! [`ParsedPacket`] variants by matching on the opcode byte at index 4.
//!
//! References:
//! - `AACPManager.kt` `receivePacket()`
//! - `main.cpp` `parseData()`

use crate::device::battery::{self, BatteryInfo};
use crate::device::state::NoiseControlMode;
use crate::protocol::control_command::{ControlCommand, ControlCommandId};
use crate::protocol::opcodes;
use crate::protocol::packets;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("packet too short ({0} bytes)")]
    TooShort(usize),

    #[error("missing AAP header")]
    BadHeader,

    #[error("unknown opcode 0x{0:02X}")]
    UnknownOpcode(u8),

    #[error("battery parse error: {0}")]
    Battery(#[from] battery::BatteryParseError),

    #[error("invalid metadata: {0}")]
    Metadata(String),

    #[error("invalid magic cloud keys: {0}")]
    MagicCloudKeys(String),
}

// ---------------------------------------------------------------------------
// Proximity key types
// ---------------------------------------------------------------------------

/// Proximity key type in a keys-response TLV.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ProximityKeyType {
    Irk = 0x01,
    EncKey = 0x04,
}

impl ProximityKeyType {
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            0x01 => Some(Self::Irk),
            0x04 => Some(Self::EncKey),
            _ => None,
        }
    }
}

/// A single proximity key extracted from a keys-response packet.
#[derive(Debug, Clone)]
pub struct ProximityKey {
    pub key_type: ProximityKeyType,
    pub data: Vec<u8>,
}

// ---------------------------------------------------------------------------
// Parsed packet enum
// ---------------------------------------------------------------------------

/// Result of parsing an incoming AAP packet.
#[derive(Debug)]
pub enum ParsedPacket {
    /// Battery status for left, right, case.
    Battery(BatteryInfo),

    /// Ear detection: `true` = in ear, `false` = out / in case.
    EarDetection {
        primary_in_ear: bool,
        secondary_in_ear: bool,
    },

    /// Noise control mode notification from AirPods (stem press or remote set).
    NoiseControl(NoiseControlMode),

    /// Conversational awareness data byte.
    ConversationalAwareness { status: u8 },

    /// A control command received from AirPods.
    ControlCommand { id: ControlCommandId, data: [u8; 4] },

    /// Device metadata (name, model number, manufacturer).
    Metadata {
        name: String,
        model_number: String,
        manufacturer: String,
    },

    /// Head tracking sensor data (raw payload after header).
    HeadTracking(Vec<u8>),

    /// Proximity keys response.
    ProximityKeys(Vec<ProximityKey>),

    /// Handshake ACK — signals that SET_SPECIFIC_FEATURES should be sent.
    HandshakeAck,

    /// Features ACK — signals that REQUEST_NOTIFICATIONS should be sent.
    FeaturesAck,

    /// Magic cloud keys (IRK + EncKey, 16 bytes each).
    MagicCloudKeys { irk: [u8; 16], enc_key: [u8; 16] },

    /// Unrecognised packet.
    Unknown(Vec<u8>),
}

// ---------------------------------------------------------------------------
// Top-level parse function
// ---------------------------------------------------------------------------

/// Parse a raw AAP packet received from the L2CAP socket.
pub fn parse(packet: &[u8]) -> Result<ParsedPacket, ParseError> {
    if packet.len() < 4 {
        return Err(ParseError::TooShort(packet.len()));
    }

    // Handshake ACK has a different header prefix
    if packet.starts_with(&packets::HANDSHAKE_ACK_PREFIX) {
        return Ok(ParsedPacket::HandshakeAck);
    }

    // Features ACK
    if packet.starts_with(&packets::FEATURES_ACK_PREFIX) {
        return Ok(ParsedPacket::FeaturesAck);
    }

    // Magic cloud keys response
    if packet.starts_with(&packets::MAGIC_CLOUD_KEYS_HEADER) {
        return parse_magic_cloud_keys(packet);
    }

    // Standard packets must start with the AAP header
    if packet[..4] != packets::HEADER {
        return Err(ParseError::BadHeader);
    }

    if packet.len() < 6 {
        return Err(ParseError::TooShort(packet.len()));
    }

    let opcode = packet[4];

    match opcode {
        opcodes::BATTERY_INFO => {
            let info = battery::parse_battery(packet)?;
            Ok(ParsedPacket::Battery(info))
        }

        opcodes::EAR_DETECTION => {
            if packet.len() < 8 {
                return Err(ParseError::TooShort(packet.len()));
            }
            Ok(ParsedPacket::EarDetection {
                primary_in_ear: packet[6] == 0x00,
                secondary_in_ear: packet[7] == 0x00,
            })
        }

        opcodes::CONTROL_COMMAND => parse_control_command(packet),

        opcodes::CONVERSATION_AWARENESS => {
            // CA status data: 04 00 04 00 4B 00 02 00 XX YY
            // The status/toggle notification also uses opcode 0x4B.
            if packet.len() >= 10 && packet.starts_with(&packets::CA_DATA_PREFIX) {
                Ok(ParsedPacket::ConversationalAwareness { status: packet[9] })
            } else {
                // It might be a control command-style CA packet (via opcode 0x09
                // with id 0x28). Fall through to Unknown.
                Ok(ParsedPacket::Unknown(packet.to_vec()))
            }
        }

        opcodes::DEVICE_METADATA => parse_metadata(packet),

        opcodes::HEADTRACKING => {
            if packet.len() < 70 {
                return Err(ParseError::TooShort(packet.len()));
            }
            Ok(ParsedPacket::HeadTracking(packet.to_vec()))
        }

        opcodes::PROXIMITY_KEYS_RSP => parse_proximity_keys(packet),

        _ => Ok(ParsedPacket::Unknown(packet.to_vec())),
    }
}

// ---------------------------------------------------------------------------
// Sub-parsers
// ---------------------------------------------------------------------------

/// Parse a control command notification from AirPods.
fn parse_control_command(packet: &[u8]) -> Result<ParsedPacket, ParseError> {
    if packet.len() < 11 {
        return Err(ParseError::TooShort(packet.len()));
    }

    let id_byte = packet[6];
    let id = match ControlCommandId::from_byte(id_byte) {
        Some(id) => id,
        None => {
            log::warn!("Unknown control command identifier: 0x{:02X}", id_byte);
            return Ok(ParsedPacket::Unknown(packet.to_vec()));
        }
    };

    // If it's a noise control mode change, also return a typed variant
    if id == ControlCommandId::ListeningMode {
        let mode = match NoiseControlMode::from_byte(packet[7]) {
            Some(m) => m,
            None => return Ok(ParsedPacket::Unknown(packet.to_vec())),
        };
        return Ok(ParsedPacket::NoiseControl(mode));
    }

    let data = ControlCommand::parse_data(packet).unwrap_or([0; 4]);
    Ok(ParsedPacket::ControlCommand { id, data })
}

/// Parse device metadata: null-terminated strings for name, model number,
/// manufacturer.
///
/// Layout: `04 00 04 00 1D [6 skip bytes] name\0 model\0 manufacturer\0`
fn parse_metadata(packet: &[u8]) -> Result<ParsedPacket, ParseError> {
    if !packet.starts_with(&packets::METADATA_PREFIX) {
        return Err(ParseError::Metadata("bad header".into()));
    }

    let skip = packets::METADATA_PREFIX.len() + 6;
    if packet.len() < skip + 3 {
        return Err(ParseError::Metadata("too short".into()));
    }

    let data = &packet[skip..];
    let mut strings = Vec::new();
    let mut start = 0;
    for i in 0..data.len() {
        if data[i] == 0x00 {
            let s = String::from_utf8_lossy(&data[start..i]).to_string();
            strings.push(s);
            start = i + 1;
            if strings.len() == 3 {
                break;
            }
        }
    }

    // Pad with empty strings if fewer than 3 null-terminated strings found
    while strings.len() < 3 {
        if start < data.len() {
            strings.push(String::from_utf8_lossy(&data[start..]).to_string());
            start = data.len();
        } else {
            strings.push(String::new());
        }
    }

    Ok(ParsedPacket::Metadata {
        name: strings[0].clone(),
        model_number: strings[1].clone(),
        manufacturer: strings[2].clone(),
    })
}

/// Parse proximity keys response.
///
/// Layout: `04 00 04 00 31 00 [count] ([type] [len_hi] [len_lo] [reserved] [key_bytes...])*`
fn parse_proximity_keys(packet: &[u8]) -> Result<ParsedPacket, ParseError> {
    if packet.len() < 7 {
        return Err(ParseError::TooShort(packet.len()));
    }

    let count = packet[6] as usize;
    let mut keys = Vec::with_capacity(count);
    let mut offset = 7;

    for _ in 0..count {
        if offset + 4 > packet.len() {
            break;
        }
        let key_type_byte = packet[offset];
        let key_len = packet[offset + 2] as usize;
        offset += 4; // type + len_hi + len_lo + reserved

        if offset + key_len > packet.len() {
            break;
        }

        let data = packet[offset..offset + key_len].to_vec();
        offset += key_len;

        if let Some(kt) = ProximityKeyType::from_byte(key_type_byte) {
            keys.push(ProximityKey { key_type: kt, data });
        }
    }

    Ok(ParsedPacket::ProximityKeys(keys))
}

/// Parse magic cloud keys response.
///
/// Header: `04 00 04 00 31 00 02`, then two 16-byte TLV blocks.
fn parse_magic_cloud_keys(packet: &[u8]) -> Result<ParsedPacket, ParseError> {
    if packet.len() < 47 {
        return Err(ParseError::TooShort(packet.len()));
    }

    let mut idx = packets::MAGIC_CLOUD_KEYS_HEADER.len();

    // First TLV block — IRK (type 0x01)
    if packet[idx] != 0x01 {
        return Err(ParseError::MagicCloudKeys("expected IRK type 0x01".into()));
    }
    idx += 1;
    let len1 = ((packet[idx] as u16) << 8) | packet[idx + 1] as u16;
    if len1 != 16 {
        return Err(ParseError::MagicCloudKeys(format!(
            "IRK length {len1}, expected 16"
        )));
    }
    idx += 3; // len (2 bytes) + reserved (1 byte)
    let mut irk = [0u8; 16];
    irk.copy_from_slice(&packet[idx..idx + 16]);
    idx += 16;

    // Second TLV block — EncKey (type 0x04)
    if idx >= packet.len() || packet[idx] != 0x04 {
        return Err(ParseError::MagicCloudKeys("expected EncKey type 0x04".into()));
    }
    idx += 1;
    let len2 = ((packet[idx] as u16) << 8) | packet[idx + 1] as u16;
    if len2 != 16 {
        return Err(ParseError::MagicCloudKeys(format!(
            "EncKey length {len2}, expected 16"
        )));
    }
    idx += 3;
    let mut enc_key = [0u8; 16];
    if idx + 16 > packet.len() {
        return Err(ParseError::TooShort(packet.len()));
    }
    enc_key.copy_from_slice(&packet[idx..idx + 16]);

    Ok(ParsedPacket::MagicCloudKeys { irk, enc_key })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_handshake_ack() {
        let pkt = [0x01, 0x00, 0x04, 0x00, 0x01, 0x02];
        match parse(&pkt).unwrap() {
            ParsedPacket::HandshakeAck => {}
            other => panic!("expected HandshakeAck, got {:?}", other),
        }
    }

    #[test]
    fn parse_ear_detection() {
        // Primary in ear (0x00), secondary out (0x01)
        let pkt = [0x04, 0x00, 0x04, 0x00, 0x06, 0x00, 0x00, 0x01];
        match parse(&pkt).unwrap() {
            ParsedPacket::EarDetection {
                primary_in_ear,
                secondary_in_ear,
            } => {
                assert!(primary_in_ear);
                assert!(!secondary_in_ear);
            }
            other => panic!("expected EarDetection, got {:?}", other),
        }
    }

    #[test]
    fn parse_noise_control() {
        // Noise control: adaptive (0x04)
        let pkt = [
            0x04, 0x00, 0x04, 0x00, 0x09, 0x00, 0x0D, 0x04, 0x00, 0x00, 0x00,
        ];
        match parse(&pkt).unwrap() {
            ParsedPacket::NoiseControl(mode) => {
                assert_eq!(mode, NoiseControlMode::Adaptive);
            }
            other => panic!("expected NoiseControl, got {:?}", other),
        }
    }

    #[test]
    fn parse_features_ack() {
        let pkt = [0x04, 0x00, 0x04, 0x00, 0x2B, 0x00, 0x01];
        match parse(&pkt).unwrap() {
            ParsedPacket::FeaturesAck => {}
            other => panic!("expected FeaturesAck, got {:?}", other),
        }
    }

    #[test]
    fn parse_battery_packet() {
        // Example from AAP Definitions.md
        let pkt: Vec<u8> = vec![
            0x04, 0x00, 0x04, 0x00, 0x04, 0x00, 0x03, // count = 3
            0x02, 0x01, 0x64, 0x02, 0x01, // Right: 100%, discharging
            0x04, 0x01, 0x63, 0x01, 0x01, // Left: 99%, charging
            0x08, 0x01, 0x11, 0x02, 0x01, // Case: 17%, discharging
        ];
        match parse(&pkt).unwrap() {
            ParsedPacket::Battery(info) => {
                assert_eq!(info.left.level, 99);
                assert_eq!(info.right.level, 100);
                assert_eq!(info.case_.level, 17);
            }
            other => panic!("expected Battery, got {:?}", other),
        }
    }
}
