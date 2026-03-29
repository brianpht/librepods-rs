// LibrePods - AirPods liberated from Apple's ecosystem
// Copyright (C) 2025 LibrePods contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! BLE advertisement data decoder for Apple AirPods.
//!
//! Apple AirPods broadcast Proximity Pairing Messages via BLE manufacturer
//! data (company ID `0x004C`). This module decodes the raw bytes into a
//! structured [`BleAdvertData`].
//!
//! The parsing logic is currently **duplicated** across:
//! - `BLEManager.kt` `processScanResult()` / `parseProximityMessage()`
//! - `blemanager.cpp` `onDeviceDiscovered()`
//!
//! This module unifies both implementations into a single cross-platform parser.

use serde::{Deserialize, Serialize};

use crate::device::state::{model_from_ble_id, AirPodsModel};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Connection state decoded from BLE ad byte 10.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectionState {
    Disconnected,
    Idle,
    Music,
    Call,
    Ringing,
    HangingUp,
    Unknown(u8),
}

impl ConnectionState {
    pub fn from_byte(b: u8) -> Self {
        match b {
            0x00 => Self::Disconnected,
            0x04 => Self::Idle,
            0x05 => Self::Music,
            0x06 => Self::Call,
            0x07 => Self::Ringing,
            0x09 => Self::HangingUp,
            other => Self::Unknown(other),
        }
    }

    pub fn as_byte(self) -> u8 {
        match self {
            Self::Disconnected => 0x00,
            Self::Idle => 0x04,
            Self::Music => 0x05,
            Self::Call => 0x06,
            Self::Ringing => 0x07,
            Self::HangingUp => 0x09,
            Self::Unknown(b) => b,
        }
    }
}

/// Lid state of the AirPods case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LidState {
    Open,
    Closed,
    Unknown,
}

/// Decoded AirPods BLE advertisement data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BleAdvertData {
    pub paired: bool,
    pub model_id: u16,
    pub model: AirPodsModel,

    // Battery (from unencrypted nibbles — 10% resolution)
    pub left_battery: Option<u8>,
    pub right_battery: Option<u8>,
    pub case_battery: Option<u8>,

    // Charging flags
    pub left_charging: bool,
    pub right_charging: bool,
    pub case_charging: bool,

    // Ear detection
    pub left_in_ear: bool,
    pub right_in_ear: bool,

    // Case state
    pub lid_state: LidState,
    pub one_pod_in_case: bool,
    pub both_pods_in_case: bool,

    // Device info
    pub color: u8,
    pub connection_state: ConnectionState,

    // Encrypted battery (higher resolution, requires enc_key)
    pub encrypted_left_battery: Option<u8>,
    pub encrypted_right_battery: Option<u8>,
    pub encrypted_case_battery: Option<u8>,
    pub encrypted_left_charging: Option<bool>,
    pub encrypted_right_charging: Option<bool>,
    pub encrypted_case_charging: Option<bool>,
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Parse Apple manufacturer data (`0x004C`) from a BLE advertisement.
///
/// `data` should be the raw manufacturer-specific bytes (without the company
/// ID prefix).
///
/// Returns `None` if the data is not a Proximity Pairing Message or is too
/// short.
pub fn parse_advertisement(data: &[u8]) -> Option<BleAdvertData> {
    // Minimum: type(1) + len(1) + paired(1) + model(2) + status(1) + pods_bat(1)
    //          + flags_case(1) + lid(1) + color(1) + conn(1) = 11 bytes
    if data.len() < 11 {
        return None;
    }

    // data[0] must be 0x07 = Proximity Pairing Message
    if data[0] != 0x07 {
        return None;
    }

    // data[2]: 0x00 = pairing mode (skip), 0x01 = paired
    if data[2] == 0x00 {
        return None; // pairing mode — different layout
    }
    let paired = data[2] == 0x01;

    // Model ID (big-endian)
    let model_id = ((data[3] as u16) << 8) | (data[4] as u16);
    let model = model_from_ble_id(model_id);

    // Status byte
    let status = data[5];
    let primary_left = (status & 0x20) != 0; // bit 5
    let this_in_case = (status & 0x40) != 0; // bit 6
    let one_pod_in_case = (status & 0x10) != 0; // bit 4
    let both_pods_in_case = (status & 0x04) != 0; // bit 2

    let are_values_flipped = !primary_left;

    // XOR factor for ear detection
    let xor_factor = primary_left ^ this_in_case;

    let left_in_ear = if xor_factor {
        (status & 0x08) != 0
    } else {
        (status & 0x02) != 0
    };
    let right_in_ear = if xor_factor {
        (status & 0x02) != 0
    } else {
        (status & 0x08) != 0
    };

    // Battery nibbles (10% resolution, 0xF = unavailable)
    let pods_byte = data[6];
    let left_nibble = if are_values_flipped {
        (pods_byte >> 4) & 0x0F
    } else {
        pods_byte & 0x0F
    };
    let right_nibble = if are_values_flipped {
        pods_byte & 0x0F
    } else {
        (pods_byte >> 4) & 0x0F
    };

    let left_battery = if left_nibble == 0x0F {
        None
    } else {
        Some(left_nibble * 10)
    };
    let right_battery = if right_nibble == 0x0F {
        None
    } else {
        Some(right_nibble * 10)
    };

    // Case battery + charging flags
    let flags_case = data[7];
    let case_nibble = flags_case & 0x0F;
    let case_battery = if case_nibble == 0x0F {
        None
    } else {
        Some(case_nibble * 10)
    };

    let flags = (flags_case >> 4) & 0x0F;
    let right_charging = if are_values_flipped {
        (flags & 0x01) != 0
    } else {
        (flags & 0x02) != 0
    };
    let left_charging = if are_values_flipped {
        (flags & 0x02) != 0
    } else {
        (flags & 0x01) != 0
    };
    let case_charging = (flags & 0x04) != 0;

    // Lid indicator
    let lid_byte = data[8];
    let lid_bit = (lid_byte >> 3) & 0x01;
    let lid_state = if this_in_case {
        if lid_bit == 0 {
            LidState::Open
        } else {
            LidState::Closed
        }
    } else {
        LidState::Unknown
    };

    let color = data[9];
    let connection_state = ConnectionState::from_byte(data[10]);

    let mut advert = BleAdvertData {
        paired,
        model_id,
        model,
        left_battery,
        right_battery,
        case_battery,
        left_charging,
        right_charging,
        case_charging,
        left_in_ear,
        right_in_ear,
        lid_state,
        one_pod_in_case,
        both_pods_in_case,
        color,
        connection_state,
        encrypted_left_battery: None,
        encrypted_right_battery: None,
        encrypted_case_battery: None,
        encrypted_left_charging: None,
        encrypted_right_charging: None,
        encrypted_case_charging: None,
    };

    // If there is a 16-byte encrypted payload at the end and a key is provided
    // externally, the caller can use `decrypt_battery` to fill in encrypted_*
    // fields. We don't do decryption here to keep the function pure and not
    // require the key as a parameter.
    let _ = &mut advert; // suppress unused mut warning

    Some(advert)
}

/// Decrypt the encrypted battery payload and fill in the high-resolution fields.
///
/// `encrypted_payload` should be exactly 16 bytes (the last 16 bytes of the
/// manufacturer data). `enc_key` is the 16-byte AES key from proximity keys.
///
/// Returns the decrypted block on success, `None` on failure.
pub fn decrypt_battery(
    advert: &mut BleAdvertData,
    encrypted_payload: &[u8; 16],
    enc_key: &[u8; 16],
    primary_left: bool,
) -> Option<()> {
    use aes::cipher::{BlockDecrypt, KeyInit};
    use aes::Aes128;

    let cipher = Aes128::new(enc_key.into());
    let mut block = aes::Block::clone_from_slice(encrypted_payload);
    cipher.decrypt_block(&mut block);
    let decrypted = block.as_slice();

    let is_flipped = !primary_left;
    let left_idx: usize = if is_flipped { 2 } else { 1 };
    let right_idx: usize = if is_flipped { 1 } else { 2 };

    let parse_bat = |b: u8| -> (bool, u8) {
        let charging = (b & 0x80) != 0;
        let level = b & 0x7F;
        (charging, level)
    };

    let (lc, ll) = parse_bat(decrypted[left_idx]);
    let (rc, rl) = parse_bat(decrypted[right_idx]);
    let (cc, cl) = parse_bat(decrypted[3]);

    advert.encrypted_left_battery = Some(ll);
    advert.encrypted_right_battery = Some(rl);
    advert.encrypted_left_charging = Some(lc);
    advert.encrypted_right_charging = Some(rc);

    // Case battery: 0xFF or (charging && level==127) means unavailable
    let raw = decrypted[3];
    if raw != 0xFF && !(cc && cl == 127) {
        advert.encrypted_case_battery = Some(cl);
        advert.encrypted_case_charging = Some(cc);
    }

    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_advertisement() {
        // Minimal 11-byte proximity pairing message
        let data: [u8; 11] = [
            0x07, // type
            0x19, // length
            0x01, // paired
            0x24, 0x20, // model = 0x2420 = AirPods Pro 2 USB-C
            0x20, // status: primaryLeft=1 (bit5), others 0
            0x98, // pods battery: upper=9, lower=8 => left=80%, right=90%
            0x14, // flags=1 (upper nibble), case=4 (lower) => case=40%
            0x00, // lid
            0x00, // color = white
            0x04, // conn = Idle
        ];

        let ad = parse_advertisement(&data).unwrap();
        assert_eq!(ad.model, AirPodsModel::AirPodsPro2Usbc);
        assert!(ad.paired);
        assert_eq!(ad.left_battery, Some(80));
        assert_eq!(ad.right_battery, Some(90));
        assert_eq!(ad.case_battery, Some(40));
        assert_eq!(ad.connection_state, ConnectionState::Idle);
    }

    #[test]
    fn skip_pairing_mode() {
        let mut data = [0u8; 11];
        data[0] = 0x07;
        data[2] = 0x00; // pairing mode
        assert!(parse_advertisement(&data).is_none());
    }

    #[test]
    fn skip_non_proximity() {
        let data = [0x01; 11]; // wrong type byte
        assert!(parse_advertisement(&data).is_none());
    }
}
