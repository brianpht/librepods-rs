// LibrePods - AirPods liberated from Apple's ecosystem
// Copyright (C) 2025 LibrePods contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Device state model, AirPods model enum, noise control mode.
//!
//! References:
//! - `deviceinfo.hpp` — `DeviceInfo` QObject
//! - `enums.h` — `NoiseControlMode`, `AirPodsModel`, `parseModelNumber()`

use serde::{Deserialize, Serialize};

use crate::device::battery::BatteryInfo;

// ---------------------------------------------------------------------------
// Noise control
// ---------------------------------------------------------------------------

/// Active noise control mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum NoiseControlMode {
    Off = 0x01,
    NoiseCancellation = 0x02,
    Transparency = 0x03,
    Adaptive = 0x04,
}

impl NoiseControlMode {
    /// Wire value used in AAP control commands (ListeningMode identifier 0x0D).
    pub fn as_byte(self) -> u8 {
        self as u8
    }

    /// Parse from wire value.
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            0x01 => Some(Self::Off),
            0x02 => Some(Self::NoiseCancellation),
            0x03 => Some(Self::Transparency),
            0x04 => Some(Self::Adaptive),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// AirPods model
// ---------------------------------------------------------------------------

/// Known AirPods hardware models.
///
/// Model numbers sourced from <https://support.apple.com/en-us/109525>
/// and `enums.h` `parseModelNumber()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum AirPodsModel {
    #[default]
    Unknown,
    AirPods1,
    AirPods2,
    AirPods3,
    AirPodsPro,
    AirPodsPro2Lightning,
    AirPodsPro2Usbc,
    AirPodsMaxLightning,
    AirPodsMaxUsbc,
    AirPods4,
    AirPods4Anc,
}

/// Map an Apple model number string (e.g. `"A3047"`) to an [`AirPodsModel`].
pub fn model_from_number(model_number: &str) -> AirPodsModel {
    match model_number {
        "A1523" | "A1722" => AirPodsModel::AirPods1,
        "A2032" | "A2031" => AirPodsModel::AirPods2,
        "A2565" | "A2564" => AirPodsModel::AirPods3,
        "A2084" | "A2083" => AirPodsModel::AirPodsPro,
        "A2931" | "A2699" | "A2698" => AirPodsModel::AirPodsPro2Lightning,
        "A3047" | "A3048" | "A3049" => AirPodsModel::AirPodsPro2Usbc,
        "A2096" => AirPodsModel::AirPodsMaxLightning,
        "A3184" => AirPodsModel::AirPodsMaxUsbc,
        "A3053" | "A3050" | "A3054" => AirPodsModel::AirPods4,
        "A3056" | "A3055" | "A3057" => AirPodsModel::AirPods4Anc,
        _ => AirPodsModel::Unknown,
    }
}

/// Map a BLE advertisement model ID (big-endian u16) to an [`AirPodsModel`].
///
/// Source: `BLEManager.kt` `modelNames` map.
pub fn model_from_ble_id(id: u16) -> AirPodsModel {
    match id {
        0x0220 => AirPodsModel::AirPods1,
        0x0F20 => AirPodsModel::AirPods2,
        0x1320 => AirPodsModel::AirPods3,
        0x0E20 => AirPodsModel::AirPodsPro,
        0x1420 => AirPodsModel::AirPodsPro2Lightning,
        0x2420 => AirPodsModel::AirPodsPro2Usbc,
        0x0A20 => AirPodsModel::AirPodsMaxLightning,
        0x1F20 => AirPodsModel::AirPodsMaxUsbc,
        0x1920 => AirPodsModel::AirPods4,
        0x1B20 => AirPodsModel::AirPods4Anc,
        _ => AirPodsModel::Unknown,
    }
}

// ---------------------------------------------------------------------------
// Device state
// ---------------------------------------------------------------------------

/// Aggregate device state — the central "truth" struct updated by the parser.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceState {
    pub device_name: String,
    pub model_number: String,
    pub manufacturer: String,
    pub model: AirPodsModel,
    pub bluetooth_address: String,

    pub battery: BatteryInfo,
    pub noise_control: NoiseControlMode,
    pub conversational_awareness: bool,
    pub primary_in_ear: bool,
    pub secondary_in_ear: bool,
    pub adaptive_noise_level: u8,
    pub one_bud_anc: bool,

    /// Identity Resolving Key for BLE RPA verification (16 bytes).
    pub magic_acc_irk: Option<[u8; 16]>,
    /// Encryption key for BLE ad battery decryption (16 bytes).
    pub magic_acc_enc_key: Option<[u8; 16]>,
}

impl Default for DeviceState {
    fn default() -> Self {
        Self {
            device_name: String::new(),
            model_number: String::new(),
            manufacturer: String::new(),
            model: AirPodsModel::Unknown,
            bluetooth_address: String::new(),
            battery: BatteryInfo::default(),
            noise_control: NoiseControlMode::Off,
            conversational_awareness: false,
            primary_in_ear: false,
            secondary_in_ear: false,
            adaptive_noise_level: 50,
            one_bud_anc: false,
            magic_acc_irk: None,
            magic_acc_enc_key: None,
        }
    }
}

impl DeviceState {
    /// At least one pod is in ear.
    pub fn any_pod_in_ear(&self) -> bool {
        self.primary_in_ear || self.secondary_in_ear
    }

    /// Both pods are in ear.
    pub fn both_pods_in_ear(&self) -> bool {
        self.primary_in_ear && self.secondary_in_ear
    }

    /// Reset all state (e.g. on disconnect).
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_lookup() {
        assert_eq!(model_from_number("A3047"), AirPodsModel::AirPodsPro2Usbc);
        assert_eq!(model_from_number("XXXX"), AirPodsModel::Unknown);
    }

    #[test]
    fn ble_model_lookup() {
        assert_eq!(model_from_ble_id(0x2420), AirPodsModel::AirPodsPro2Usbc);
        assert_eq!(model_from_ble_id(0xFFFF), AirPodsModel::Unknown);
    }

    #[test]
    fn noise_control_round_trip() {
        for mode in [
            NoiseControlMode::Off,
            NoiseControlMode::NoiseCancellation,
            NoiseControlMode::Transparency,
            NoiseControlMode::Adaptive,
        ] {
            assert_eq!(NoiseControlMode::from_byte(mode.as_byte()), Some(mode));
        }
    }
}
