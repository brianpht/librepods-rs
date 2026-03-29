// LibrePods - AirPods liberated from Apple's ecosystem
// Copyright (C) 2025 LibrePods contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! ListeningModeConfigs bitmask builder.
//!
//! Replaces the ~40 hardcoded `LongPressPackets` enum variants in `Packets.kt`
//! with a single function that computes the bitmask dynamically.
//!
//! The `ListeningModeConfigs` control command (identifier `0x1A`) takes a
//! single byte whose bits select which noise-control modes are available via
//! the long-press gesture:
//!
//! | Bit | Mode              |
//! |-----|-------------------|
//! | 0   | Off        (0x01) |
//! | 1   | ANC        (0x02) |
//! | 2   | Transparency(0x04)|
//! | 3   | Adaptive   (0x08) |
//!
//! Reference: `docs/control_commands.md` identifier `0x1A`.

use crate::device::state::NoiseControlMode;
use crate::protocol::control_command::ControlCommand;

/// Bitmask value for each mode in the `ListeningModeConfigs` byte.
pub fn mode_bit(mode: NoiseControlMode) -> u8 {
    match mode {
        NoiseControlMode::Off => 0x01,
        NoiseControlMode::NoiseCancellation => 0x02,
        NoiseControlMode::Transparency => 0x04,
        NoiseControlMode::Adaptive => 0x08,
    }
}

/// Build a `ListeningModeConfigs` control command packet (11 bytes) that
/// enables exactly the given set of modes for the long-press cycle.
///
/// ```
/// use librepods_core::device::state::NoiseControlMode;
/// use librepods_core::device::listening_mode::build_listening_mode_config;
///
/// let pkt = build_listening_mode_config(&[
///     NoiseControlMode::NoiseCancellation,
///     NoiseControlMode::Transparency,
///     NoiseControlMode::Adaptive,
/// ]);
/// assert_eq!(pkt[6], 0x1A); // identifier
/// assert_eq!(pkt[7], 0x0E); // 0x02 | 0x04 | 0x08
/// ```
pub fn build_listening_mode_config(modes: &[NoiseControlMode]) -> [u8; 11] {
    let mut bitmask: u8 = 0;
    for &mode in modes {
        bitmask |= mode_bit(mode);
    }
    ControlCommand::create(0x1A, &[bitmask, 0x00, 0x00, 0x00])
}

/// Parse a `ListeningModeConfigs` byte back into a list of enabled modes.
pub fn parse_listening_mode_config(bitmask: u8) -> Vec<NoiseControlMode> {
    let mut modes = Vec::new();
    if bitmask & 0x01 != 0 {
        modes.push(NoiseControlMode::Off);
    }
    if bitmask & 0x02 != 0 {
        modes.push(NoiseControlMode::NoiseCancellation);
    }
    if bitmask & 0x04 != 0 {
        modes.push(NoiseControlMode::Transparency);
    }
    if bitmask & 0x08 != 0 {
        modes.push(NoiseControlMode::Adaptive);
    }
    modes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_modes() {
        let pkt = build_listening_mode_config(&[
            NoiseControlMode::Off,
            NoiseControlMode::NoiseCancellation,
            NoiseControlMode::Transparency,
            NoiseControlMode::Adaptive,
        ]);
        assert_eq!(pkt[6], 0x1A);
        assert_eq!(pkt[7], 0x0F); // 0x01 | 0x02 | 0x04 | 0x08
    }

    #[test]
    fn without_off() {
        let pkt = build_listening_mode_config(&[
            NoiseControlMode::NoiseCancellation,
            NoiseControlMode::Transparency,
            NoiseControlMode::Adaptive,
        ]);
        assert_eq!(pkt[7], 0x0E);
    }

    #[test]
    fn round_trip() {
        let modes = vec![
            NoiseControlMode::Off,
            NoiseControlMode::Transparency,
            NoiseControlMode::Adaptive,
        ];
        let pkt = build_listening_mode_config(&modes);
        let parsed = parse_listening_mode_config(pkt[7]);
        assert_eq!(parsed, modes);
    }
}
