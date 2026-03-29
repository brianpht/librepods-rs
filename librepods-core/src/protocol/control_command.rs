// LibrePods - AirPods liberated from Apple's ecosystem
// Copyright (C) 2025 LibrePods contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Generic control command builder and all known command identifiers.
//!
//! AAP control commands have the fixed layout (11 bytes total):
//! ```text
//! 04 00 04 00 09 00 [identifier] [data1] [data2] [data3] [data4]
//! ```
//!
//! References:
//! - `BasicControlCommand.hpp` — C++ template for `create`, `enabled`, `disabled`, `parseState`
//! - `AACPManager.kt` `ControlCommandIdentifiers` enum (0x01–0x34)
//! - `docs/control_commands.md`

use serde::{Deserialize, Serialize};

/// The 6-byte header shared by every control command packet.
pub const CONTROL_COMMAND_HEADER: [u8; 6] = [0x04, 0x00, 0x04, 0x00, 0x09, 0x00];

/// All known control command identifiers.
///
/// Source: `AACPManager.kt` `ControlCommandIdentifiers`, `docs/control_commands.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum ControlCommandId {
    MicMode = 0x01,
    ButtonSendMode = 0x05,
    ListeningMode = 0x0D,
    VoiceTrigger = 0x12,
    SingleClickMode = 0x14,
    DoubleClickMode = 0x15,
    ClickHoldMode = 0x16,
    DoubleClickInterval = 0x17,
    ClickHoldInterval = 0x18,
    ListeningModeConfigs = 0x1A,
    OneBudAncMode = 0x1B,
    CrownRotationDirection = 0x1C,
    AutoAnswerMode = 0x1E,
    ChimeVolume = 0x1F,
    VolumeSwipeInterval = 0x23,
    CallManagementConfig = 0x24,
    VolumeSwipeMode = 0x25,
    AdaptiveVolumeConfig = 0x26,
    SoftwareMuteConfig = 0x27,
    ConversationDetectConfig = 0x28,
    Ssl = 0x29,
    HearingAid = 0x2C,
    AutoAncStrength = 0x2E,
    HpsGainSwipe = 0x2F,
    HrmState = 0x30,
    InCaseToneConfig = 0x31,
    SiriMultitoneConfig = 0x32,
    HearingAssistConfig = 0x33,
    AllowOffOption = 0x34,
}

impl ControlCommandId {
    /// Look up an identifier by its byte value.
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            0x01 => Some(Self::MicMode),
            0x05 => Some(Self::ButtonSendMode),
            0x0D => Some(Self::ListeningMode),
            0x12 => Some(Self::VoiceTrigger),
            0x14 => Some(Self::SingleClickMode),
            0x15 => Some(Self::DoubleClickMode),
            0x16 => Some(Self::ClickHoldMode),
            0x17 => Some(Self::DoubleClickInterval),
            0x18 => Some(Self::ClickHoldInterval),
            0x1A => Some(Self::ListeningModeConfigs),
            0x1B => Some(Self::OneBudAncMode),
            0x1C => Some(Self::CrownRotationDirection),
            0x1E => Some(Self::AutoAnswerMode),
            0x1F => Some(Self::ChimeVolume),
            0x23 => Some(Self::VolumeSwipeInterval),
            0x24 => Some(Self::CallManagementConfig),
            0x25 => Some(Self::VolumeSwipeMode),
            0x26 => Some(Self::AdaptiveVolumeConfig),
            0x27 => Some(Self::SoftwareMuteConfig),
            0x28 => Some(Self::ConversationDetectConfig),
            0x29 => Some(Self::Ssl),
            0x2C => Some(Self::HearingAid),
            0x2E => Some(Self::AutoAncStrength),
            0x2F => Some(Self::HpsGainSwipe),
            0x30 => Some(Self::HrmState),
            0x31 => Some(Self::InCaseToneConfig),
            0x32 => Some(Self::SiriMultitoneConfig),
            0x33 => Some(Self::HearingAssistConfig),
            0x34 => Some(Self::AllowOffOption),
            _ => None,
        }
    }

    /// The raw byte value of this identifier.
    pub fn as_byte(self) -> u8 {
        self as u8
    }
}

/// Stateless helpers for building and parsing control command packets.
///
/// Mirrors `BasicControlCommand<CommandId>` from `BasicControlCommand.hpp`.
pub struct ControlCommand;

impl ControlCommand {
    /// Build an 11-byte control command packet.
    ///
    /// ```text
    /// [04 00 04 00 09 00] [id] [data1] [data2] [data3] [data4]
    /// ```
    pub fn create(id: u8, data: &[u8; 4]) -> [u8; 11] {
        let mut pkt = [0u8; 11];
        pkt[..6].copy_from_slice(&CONTROL_COMMAND_HEADER);
        pkt[6] = id;
        pkt[7] = data[0];
        pkt[8] = data[1];
        pkt[9] = data[2];
        pkt[10] = data[3];
        pkt
    }

    /// Shortcut: build a control command with `data1 = 0x01` (enabled).
    pub fn enabled(id: u8) -> [u8; 11] {
        Self::create(id, &[0x01, 0x00, 0x00, 0x00])
    }

    /// Shortcut: build a control command with `data1 = 0x02` (disabled).
    pub fn disabled(id: u8) -> [u8; 11] {
        Self::create(id, &[0x02, 0x00, 0x00, 0x00])
    }

    /// Parse the boolean state from a received control command packet.
    ///
    /// Returns `Some(true)` if byte\[7\] == 0x01, `Some(false)` if 0x02,
    /// `None` otherwise.
    pub fn parse_state(packet: &[u8]) -> Option<bool> {
        let val = Self::parse_value(packet)?;
        match val {
            0x01 => Some(true),
            0x02 => Some(false),
            _ => None,
        }
    }

    /// Extract the raw value byte (byte\[7\]) from a control command packet.
    ///
    /// Returns `None` if the packet is too short or doesn't start with the
    /// control command header.
    pub fn parse_value(packet: &[u8]) -> Option<u8> {
        if packet.len() < 8 {
            return None;
        }
        if packet[..6] != CONTROL_COMMAND_HEADER {
            return None;
        }
        Some(packet[7])
    }

    /// Extract the identifier byte (byte\[6\]) from a control command packet.
    pub fn parse_id(packet: &[u8]) -> Option<u8> {
        if packet.len() < 7 {
            return None;
        }
        if packet[..6] != CONTROL_COMMAND_HEADER {
            return None;
        }
        Some(packet[6])
    }

    /// Extract all 4 data bytes (bytes 7–10) from a control command packet.
    pub fn parse_data(packet: &[u8]) -> Option<[u8; 4]> {
        if packet.len() < 11 {
            return None;
        }
        if packet[..6] != CONTROL_COMMAND_HEADER {
            return None;
        }
        let mut data = [0u8; 4];
        data.copy_from_slice(&packet[7..11]);
        Some(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_parse_round_trip() {
        let pkt = ControlCommand::create(0x0D, &[0x02, 0x00, 0x00, 0x00]);
        assert_eq!(pkt[..6], CONTROL_COMMAND_HEADER);
        assert_eq!(pkt[6], 0x0D);
        assert_eq!(ControlCommand::parse_id(&pkt), Some(0x0D));
        assert_eq!(ControlCommand::parse_value(&pkt), Some(0x02));
        assert_eq!(ControlCommand::parse_state(&pkt), Some(false));
    }

    #[test]
    fn enabled_disabled() {
        let on = ControlCommand::enabled(0x28);
        assert_eq!(on[7], 0x01);
        assert_eq!(ControlCommand::parse_state(&on), Some(true));

        let off = ControlCommand::disabled(0x28);
        assert_eq!(off[7], 0x02);
        assert_eq!(ControlCommand::parse_state(&off), Some(false));
    }

    #[test]
    fn from_byte_all_known() {
        assert_eq!(
            ControlCommandId::from_byte(0x01),
            Some(ControlCommandId::MicMode)
        );
        assert_eq!(
            ControlCommandId::from_byte(0x34),
            Some(ControlCommandId::AllowOffOption)
        );
        assert_eq!(ControlCommandId::from_byte(0xFF), None);
    }
}
