// LibrePods - AirPods liberated from Apple's ecosystem
// Copyright (C) 2025 LibrePods contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! AAP packet construction.
//!
//! Static byte arrays for well-known packets and builder functions for
//! parameterized packets (noise control, rename, head tracking, etc.).
//!
//! References:
//! - `Packets.kt` `Enums` enum
//! - `airpods_packets.h` `Connection::*`, `NoiseControl::*`, `Rename::*`
//! - `AAP Definitions.md`

use crate::device::state::NoiseControlMode;
use crate::protocol::control_command::ControlCommand;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// 4-byte header present at the start of every AAP data packet.
pub const HEADER: [u8; 4] = [0x04, 0x00, 0x04, 0x00];

/// L2CAP PSM used by AAP.
pub const AAP_PSM: u16 = 0x1001;

/// Bluetooth service UUID that identifies AirPods.
pub const AIRPODS_UUID: &str = "74ec2172-0bad-4d01-8f77-997b2be0722a";

// ---------------------------------------------------------------------------
// Connection sequence packets
// ---------------------------------------------------------------------------

/// Handshake — must be sent first after L2CAP connection is established.
pub const HANDSHAKE: [u8; 16] = [
    0x00, 0x00, 0x04, 0x00, 0x01, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// Set specific features — enables Conversational Awareness during audio
/// playback and Adaptive Transparency. Sent after receiving handshake ACK.
pub const SET_SPECIFIC_FEATURES: [u8; 14] = [
    0x04, 0x00, 0x04, 0x00, 0x4D, 0x00, 0xFF, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// Request notifications — subscribe to battery, ear detection, noise control,
/// conversational awareness, etc. Sent after features ACK.
pub const REQUEST_NOTIFICATIONS: [u8; 10] =
    [0x04, 0x00, 0x04, 0x00, 0x0F, 0x00, 0xFF, 0xFF, 0xFF, 0xFF];

// ---------------------------------------------------------------------------
// Response detection prefixes
// ---------------------------------------------------------------------------

/// Handshake ACK prefix — `01 00 04 00`.
pub const HANDSHAKE_ACK_PREFIX: [u8; 4] = [0x01, 0x00, 0x04, 0x00];

/// Features ACK prefix — `04 00 04 00 2B 00`.
pub const FEATURES_ACK_PREFIX: [u8; 6] = [0x04, 0x00, 0x04, 0x00, 0x2B, 0x00];

/// Battery status prefix — `04 00 04 00 04 00`.
pub const BATTERY_PREFIX: [u8; 6] = [0x04, 0x00, 0x04, 0x00, 0x04, 0x00];

/// Ear detection prefix — `04 00 04 00 06 00`.
pub const EAR_DETECTION_PREFIX: [u8; 6] = [0x04, 0x00, 0x04, 0x00, 0x06, 0x00];

/// Metadata prefix — `04 00 04 00 1D`.
pub const METADATA_PREFIX: [u8; 5] = [0x04, 0x00, 0x04, 0x00, 0x1D];

/// Conversational awareness data prefix — `04 00 04 00 4B 00 02 00`.
pub const CA_DATA_PREFIX: [u8; 8] = [0x04, 0x00, 0x04, 0x00, 0x4B, 0x00, 0x02, 0x00];

/// AirPods disconnected (from phone cross-device) — `00 01 00 00`.
pub const AIRPODS_DISCONNECTED: [u8; 4] = [0x00, 0x01, 0x00, 0x00];

// ---------------------------------------------------------------------------
// Head tracking
// ---------------------------------------------------------------------------

/// Start head tracking.
pub const HEAD_TRACKING_START: [u8; 28] = [
    0x04, 0x00, 0x04, 0x00, 0x17, 0x00, 0x00, 0x00, 0x10, 0x00, 0x10, 0x00, 0x08, 0xA1, 0x02, 0x42,
    0x0B, 0x08, 0x0E, 0x10, 0x02, 0x1A, 0x05, 0x01, 0x40, 0x9C, 0x00, 0x00,
];

/// Stop head tracking.
pub const HEAD_TRACKING_STOP: [u8; 29] = [
    0x04, 0x00, 0x04, 0x00, 0x17, 0x00, 0x00, 0x00, 0x10, 0x00, 0x11, 0x00, 0x08, 0x7E, 0x10, 0x02,
    0x42, 0x0B, 0x08, 0x4E, 0x10, 0x02, 0x1A, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00,
];

// ---------------------------------------------------------------------------
// Proximity keys
// ---------------------------------------------------------------------------

/// Request proximity (magic pairing) cloud keys.
pub const REQUEST_MAGIC_CLOUD_KEYS: [u8; 8] = [0x04, 0x00, 0x04, 0x00, 0x30, 0x00, 0x05, 0x00];

/// Magic cloud keys response header.
pub const MAGIC_CLOUD_KEYS_HEADER: [u8; 7] = [0x04, 0x00, 0x04, 0x00, 0x31, 0x00, 0x02];

// ---------------------------------------------------------------------------
// Builder functions
// ---------------------------------------------------------------------------

/// Build a noise control mode packet (11 bytes).
///
/// Layout: `04 00 04 00 09 00 0D [mode] 00 00 00`
pub fn build_noise_control(mode: NoiseControlMode) -> [u8; 11] {
    ControlCommand::create(0x0D, &[mode.as_byte(), 0x00, 0x00, 0x00])
}

/// Build a rename packet.
///
/// Layout: `04 00 04 00 1A 00 01 [len] 00 [name_bytes...]`
pub fn build_rename(name: &str) -> Vec<u8> {
    let name_bytes = name.as_bytes();
    let len = name_bytes.len() as u8;
    let mut packet = Vec::with_capacity(9 + name_bytes.len());
    packet.extend_from_slice(&HEADER);
    packet.extend_from_slice(&[0x1A, 0x00, 0x01]);
    packet.push(len);
    packet.push(0x00);
    packet.extend_from_slice(name_bytes);
    packet
}

/// Build a request proximity keys packet.
///
/// `key_type_mask`: bitwise OR of key types to request (IRK=0x01, ENC_KEY=0x04).
pub fn build_request_proximity_keys(key_type_mask: u8) -> [u8; 8] {
    let mut pkt = [0u8; 8];
    pkt[..4].copy_from_slice(&HEADER);
    pkt[4] = 0x30;
    pkt[5] = 0x00;
    pkt[6] = key_type_mask;
    pkt[7] = 0x00;
    pkt
}

/// Build an adaptive noise level packet (0–100).
///
/// Layout: `04 00 04 00 09 00 2E [level] 00 00 00`
pub fn build_adaptive_noise_level(level: u8) -> [u8; 11] {
    let clamped = level.min(100);
    ControlCommand::create(0x2E, &[clamped, 0x00, 0x00, 0x00])
}

/// Check if raw data looks like a head tracking packet (size > 60, correct
/// prefix, specific tag byte at offset 10).
pub fn is_head_tracking_data(data: &[u8]) -> bool {
    if data.len() <= 60 {
        return false;
    }
    let prefix: [u8; 10] = [0x04, 0x00, 0x04, 0x00, 0x17, 0x00, 0x00, 0x00, 0x10, 0x00];
    if data[..10] != prefix {
        return false;
    }
    if data[10] != 0x44 && data[10] != 0x45 {
        return false;
    }
    data[11] == 0x00
}
