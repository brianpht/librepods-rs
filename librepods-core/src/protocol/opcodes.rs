// LibrePods - AirPods liberated from Apple's ecosystem
// Copyright (C) 2025 LibrePods contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! AAP opcode constants.
//!
//! Every AAP L2CAP packet has the format:
//! ```text
//! 04 00 04 00 [opcode_lo] [opcode_hi] [data...]
//! ```
//! The opcode sits at byte index 4 (little-endian, high byte at index 5 is
//! always 0x00 for known opcodes).
//!
//! Source: `AACPManager.kt` `Opcodes` object, `airpods_packets.h` headers.

/// Battery information notification (opcode `0x04`).
pub const BATTERY_INFO: u8 = 0x04;

/// Ear detection status (opcode `0x06`).
pub const EAR_DETECTION: u8 = 0x06;

/// Control command — covers noise control, conversational awareness toggle,
/// long-press config, chime volume, etc. (opcode `0x09`).
/// See [`super::control_command`] for identifiers.
pub const CONTROL_COMMAND: u8 = 0x09;

/// Request notifications from AirPods (opcode `0x0F`).
pub const REQUEST_NOTIFICATIONS: u8 = 0x0F;

/// Head tracking data (opcode `0x17`).
pub const HEADTRACKING: u8 = 0x17;

/// Device metadata — name, model number, manufacturer (opcode `0x1D`).
pub const DEVICE_METADATA: u8 = 0x1D;

/// Rename device (opcode `0x1E`).
pub const RENAME: u8 = 0x1E;

/// Proximity keys request (opcode `0x30`).
pub const PROXIMITY_KEYS_REQ: u8 = 0x30;

/// Proximity keys response (opcode `0x31`).
pub const PROXIMITY_KEYS_RSP: u8 = 0x31;

/// Conversational awareness data (opcode `0x4B`).
pub const CONVERSATION_AWARENESS: u8 = 0x4B;

/// Set feature flags — enables Conversational Awareness during audio playback
/// and Adaptive Transparency (opcode `0x4D`).
pub const SET_FEATURE_FLAGS: u8 = 0x4D;
