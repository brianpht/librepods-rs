// LibrePods - AirPods liberated from Apple's ecosystem
// Copyright (C) 2025 LibrePods contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Cross-device relay state machine.
//!
//! Stateless dispatcher: given an incoming cross-device packet, returns a
//! [`CrossDeviceAction`] that the platform layer should execute.
//!
//! References:
//! - `CrossDevice.kt` `handleClientConnection()`
//! - `main.cpp` `handlePhonePacket()`

use super::packets;

/// Action the platform layer should take after receiving a cross-device packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrossDeviceAction {
    /// Remote device reports AirPods are connected there.
    RemoteConnected,

    /// Remote device reports AirPods are disconnected there.
    RemoteDisconnected,

    /// Remote device asks us to disconnect our AirPods.
    DisconnectRequested,

    /// Remote device requests our battery data.
    SendBattery,

    /// Remote device requests our ANC state.
    SendAnc,

    /// Remote device requests our connection status.
    SendConnectionStatus,

    /// Relayed AirPods data from the remote device (payload after
    /// [`packets::DATA_HEADER`]).
    RelayedData(Vec<u8>),

    /// Unrecognised packet.
    Unknown(Vec<u8>),
}

/// Dispatch an incoming cross-device packet into an action.
pub fn handle_packet(packet: &[u8]) -> CrossDeviceAction {
    if packet.len() < 4 {
        return CrossDeviceAction::Unknown(packet.to_vec());
    }

    let prefix: [u8; 4] = [packet[0], packet[1], packet[2], packet[3]];

    match prefix {
        p if p == packets::CONNECTED => CrossDeviceAction::RemoteConnected,
        p if p == packets::DISCONNECTED => CrossDeviceAction::RemoteDisconnected,
        p if p == packets::REQUEST_DISCONNECT => CrossDeviceAction::DisconnectRequested,
        p if p == packets::REQUEST_BATTERY => CrossDeviceAction::SendBattery,
        p if p == packets::REQUEST_ANC => CrossDeviceAction::SendAnc,
        p if p == packets::REQUEST_STATUS => CrossDeviceAction::SendConnectionStatus,
        p if p == packets::DATA_HEADER => {
            let payload = if packet.len() > 4 {
                packet[4..].to_vec()
            } else {
                Vec::new()
            };
            CrossDeviceAction::RelayedData(payload)
        }
        _ => CrossDeviceAction::Unknown(packet.to_vec()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connected() {
        assert_eq!(
            handle_packet(&packets::CONNECTED),
            CrossDeviceAction::RemoteConnected
        );
    }

    #[test]
    fn disconnect_requested() {
        assert_eq!(
            handle_packet(&packets::REQUEST_DISCONNECT),
            CrossDeviceAction::DisconnectRequested
        );
    }

    #[test]
    fn relayed_data() {
        let mut pkt = packets::DATA_HEADER.to_vec();
        pkt.extend_from_slice(&[0x04, 0x00, 0x04, 0x00, 0x04, 0x00]);
        match handle_packet(&pkt) {
            CrossDeviceAction::RelayedData(payload) => {
                assert_eq!(payload, vec![0x04, 0x00, 0x04, 0x00, 0x04, 0x00]);
            }
            other => panic!("expected RelayedData, got {:?}", other),
        }
    }

    #[test]
    fn unknown_short() {
        assert!(matches!(
            handle_packet(&[0x01]),
            CrossDeviceAction::Unknown(_)
        ));
    }
}
