// LibrePods - AirPods liberated from Apple's ecosystem
// Copyright (C) 2025 LibrePods contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Battery packet parsing.
//!
//! AAP battery packets have the layout (22 bytes for 3 components):
//! ```text
//! 04 00 04 00 04 00 [count]
//! ([component] 01 [level] [status] 01) × count
//! ```
//!
//! References:
//! - `battery.hpp` `Battery::parsePacket()`
//! - `Packets.kt` `BatteryNotification`, `BatteryComponent`, `BatteryStatus`
//! - `AAP Definitions.md` § Battery

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::protocol::packets;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Which physical component the battery reading belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum Component {
    Right = 0x02,
    Left = 0x04,
    Case = 0x08,
}

impl Component {
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            0x02 => Some(Self::Right),
            0x04 => Some(Self::Left),
            0x08 => Some(Self::Case),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Right => "Right",
            Self::Left => "Left",
            Self::Case => "Case",
        }
    }
}

/// Charging / discharging / disconnected state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum BatteryStatus {
    Charging = 0x01,
    Discharging = 0x02,
    Disconnected = 0x04,
}

impl BatteryStatus {
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            0x01 => Some(Self::Charging),
            0x02 => Some(Self::Discharging),
            0x04 => Some(Self::Disconnected),
            _ => None,
        }
    }
}

/// Battery level + status for a single component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatteryState {
    pub level: u8,
    pub status: BatteryStatus,
}

impl Default for BatteryState {
    fn default() -> Self {
        Self {
            level: 0,
            status: BatteryStatus::Disconnected,
        }
    }
}

/// Parsed battery info for all three components.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatteryInfo {
    pub left: BatteryState,
    pub right: BatteryState,
    pub case_: BatteryState,
    /// The first pod listed in the packet is the "primary" pod.
    pub primary_pod: Component,
}

impl Default for BatteryInfo {
    fn default() -> Self {
        Self {
            left: BatteryState::default(),
            right: BatteryState::default(),
            case_: BatteryState::default(),
            primary_pod: Component::Left,
        }
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum BatteryParseError {
    #[error("packet too short ({0} bytes)")]
    TooShort(usize),

    #[error("bad header")]
    BadHeader,

    #[error("invalid component count {0}")]
    BadCount(u8),

    #[error("unknown component byte 0x{0:02X}")]
    UnknownComponent(u8),

    #[error("invalid spacer/end byte at offset {0}")]
    BadSpacer(usize),
}

/// Parse a full 22-byte battery packet starting with `04 00 04 00 04 00`.
///
/// Returns a [`BatteryInfo`] with left, right, case readings and which pod is
/// primary (appears first in the packet).
pub fn parse_battery(packet: &[u8]) -> Result<BatteryInfo, BatteryParseError> {
    if packet.len() < 7 {
        return Err(BatteryParseError::TooShort(packet.len()));
    }
    if !packet.starts_with(&packets::BATTERY_PREFIX) {
        return Err(BatteryParseError::BadHeader);
    }

    let count = packet[6];
    if count > 3 {
        return Err(BatteryParseError::BadCount(count));
    }

    let expected_len = 7 + (5 * count as usize);
    if packet.len() < expected_len {
        return Err(BatteryParseError::TooShort(packet.len()));
    }

    let mut info = BatteryInfo::default();
    let mut pods_seen: Vec<Component> = Vec::with_capacity(2);

    for i in 0..count as usize {
        let offset = 7 + (5 * i);
        let comp_byte = packet[offset];

        // Verify spacer (0x01) and end byte (0x01)
        if packet[offset + 1] != 0x01 {
            return Err(BatteryParseError::BadSpacer(offset + 1));
        }
        if packet[offset + 4] != 0x01 {
            return Err(BatteryParseError::BadSpacer(offset + 4));
        }

        let comp = Component::from_byte(comp_byte)
            .ok_or(BatteryParseError::UnknownComponent(comp_byte))?;
        let level = packet[offset + 2];
        let status =
            BatteryStatus::from_byte(packet[offset + 3]).unwrap_or(BatteryStatus::Disconnected);

        let state = BatteryState { level, status };
        match comp {
            Component::Left => info.left = state,
            Component::Right => info.right = state,
            Component::Case => info.case_ = state,
        }

        if comp == Component::Left || comp == Component::Right {
            pods_seen.push(comp);
        }
    }

    // First pod in the packet is primary
    if let Some(&first) = pods_seen.first() {
        info.primary_pod = first;
    }

    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_example_from_docs() {
        // From AAP Definitions.md:
        // 04 00 04 00 04 00 03 02 01 64 02 01 04 01 63 01 01 08 01 11 02 01
        let pkt: Vec<u8> = vec![
            0x04, 0x00, 0x04, 0x00, 0x04, 0x00, 0x03, 0x02, 0x01, 0x64, 0x02,
            0x01, // Right, 100%, discharging
            0x04, 0x01, 0x63, 0x01, 0x01, // Left, 99%, charging
            0x08, 0x01, 0x11, 0x02, 0x01, // Case, 17%, discharging
        ];
        let info = parse_battery(&pkt).unwrap();
        assert_eq!(info.right.level, 100);
        assert_eq!(info.right.status, BatteryStatus::Discharging);
        assert_eq!(info.left.level, 99);
        assert_eq!(info.left.status, BatteryStatus::Charging);
        assert_eq!(info.case_.level, 17);
        assert_eq!(info.case_.status, BatteryStatus::Discharging);
        assert_eq!(info.primary_pod, Component::Right); // Right appears first
    }
}
