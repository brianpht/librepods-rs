// LibrePods - AirPods liberated from Apple's ecosystem
// Copyright (C) 2025 LibrePods contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Platform-agnostic transport trait definitions.
//!
//! These traits define the I/O abstractions that platform crates
//! (e.g. `librepods-linux`) must implement. The core crate contains
//! **no** implementations.

use thiserror::Error;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Transport-layer error.
#[derive(Debug, Error)]
pub enum TransportError {
    /// The socket / channel is not connected.
    #[error("not connected")]
    NotConnected,

    /// The remote peer closed the connection.
    #[error("connection closed by peer")]
    ConnectionClosed,

    /// The internal channel was closed (background task ended).
    #[error("channel closed")]
    ChannelClosed,

    /// A platform I/O error with context message.
    #[error("{message}")]
    Io {
        message: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// Catch-all for messages that don't fit other variants.
    #[error("{0}")]
    Other(String),
}

impl TransportError {
    /// Convenience: create an [`Other`](Self::Other) variant.
    pub fn new(message: impl Into<String>) -> Self {
        Self::Other(message.into())
    }

    /// Convenience: create an [`Io`](Self::Io) variant with a source error.
    pub fn with_source(
        message: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::Io {
            message: message.into(),
            source: Box::new(source),
        }
    }
}

// ---------------------------------------------------------------------------
// L2CAP transport
// ---------------------------------------------------------------------------

/// Async L2CAP socket transport.
///
/// Implemented by:
/// - `librepods-linux`: raw `AF_BLUETOOTH` sockets via `nix`
pub trait L2capTransport: Send + Sync {
    /// Send raw bytes over the L2CAP connection.
    fn send(&self, data: &[u8]) -> Result<(), TransportError>;

    /// Receive bytes from the L2CAP connection. Returns number of bytes read.
    fn recv(&self, buf: &mut [u8]) -> Result<usize, TransportError>;

    /// Close the connection.
    fn close(&self) -> Result<(), TransportError>;

    /// Whether the socket is currently connected.
    fn is_connected(&self) -> bool;
}

// ---------------------------------------------------------------------------
// Device monitor
// ---------------------------------------------------------------------------

/// Event emitted by a [`DeviceMonitor`].
#[derive(Debug, Clone)]
pub enum DeviceEvent {
    /// An AirPods device connected (address, name).
    Connected { address: String, name: String },
    /// An AirPods device disconnected (address).
    Disconnected { address: String },
}

/// Watches for Bluetooth device connection/disconnection events.
///
/// Implemented by:
/// - `librepods-linux`: BlueZ D-Bus `PropertiesChanged` signal via `zbus`
pub trait DeviceMonitor: Send + Sync {
    /// Block until the next device event occurs.
    fn next_event(&self) -> Result<DeviceEvent, TransportError>;
}

// ---------------------------------------------------------------------------
// Media control
// ---------------------------------------------------------------------------

/// Platform media playback control.
///
/// Implemented by:
/// - `librepods-linux`: MPRIS D-Bus interface + PulseAudio/PipeWire
pub trait MediaControl: Send + Sync {
    fn play(&self) -> Result<(), TransportError>;
    fn pause(&self) -> Result<(), TransportError>;
    fn set_volume(&self, percent: u8) -> Result<(), TransportError>;
    fn get_volume(&self) -> Result<u8, TransportError>;
    fn is_airpods_active_sink(&self) -> bool;
}

// ---------------------------------------------------------------------------
// BLE scanner
// ---------------------------------------------------------------------------

/// Raw BLE advertisement from an Apple device.
#[derive(Debug, Clone)]
pub struct RawBleAdvertisement {
    /// Device address (may be randomized/RPA).
    pub address: String,
    /// Raw Apple manufacturer data bytes (company ID `0x004C` already stripped).
    pub manufacturer_data: Vec<u8>,
    /// RSSI if available.
    pub rssi: Option<i16>,
}

/// BLE advertisement scanner.
///
/// Implemented by:
/// - `librepods-linux`: `btleplug` (cross-platform BLE library)
pub trait BleScanner: Send + Sync {
    /// Block until the next Apple BLE advertisement is received.
    fn next_advertisement(&self) -> Result<RawBleAdvertisement, TransportError>;
}
