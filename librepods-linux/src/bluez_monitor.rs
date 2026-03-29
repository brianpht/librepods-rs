// LibrePods - AirPods liberated from Apple's ecosystem
// Copyright (C) 2025 LibrePods contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! BlueZ D-Bus device monitor.
//!
//! Watches for AirPods connection/disconnection events by subscribing to
//! `org.freedesktop.DBus.Properties.PropertiesChanged` on the system bus
//! and filtering for devices whose UUIDs contain the AirPods service UUID.
//!
//! Key design: when a device connects, BlueZ often fires `Connected = true`
//! **before** SDP service discovery has completed (so `UUIDs` is still empty).
//! We therefore also watch for `ServicesResolved` and `UUIDs` property changes
//! and keep a set of "pending" connected-but-unresolved device paths.
//!
//! Reference: `BluetoothMonitor.cpp`

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use librepods_core::protocol::packets;
use librepods_core::transport::{DeviceEvent, DeviceMonitor, TransportError};
use tokio::sync::mpsc;

/// AirPods service UUID used for device identification (lowercase).
const AIRPODS_UUID: &str = packets::AIRPODS_UUID;

/// Case-insensitive check whether `uuids` contains the AirPods service UUID.
fn contains_airpods_uuid(uuids: &[String]) -> bool {
    let target = AIRPODS_UUID.to_ascii_lowercase();
    uuids.iter().any(|u| u.to_ascii_lowercase() == target)
}

/// BlueZ D-Bus device monitor.
pub struct BluezMonitor {
    rx: Mutex<mpsc::UnboundedReceiver<DeviceEvent>>,
    /// Keep the handle alive so the background task keeps running.
    _handle: std::thread::JoinHandle<()>,
}

impl BluezMonitor {
    /// Create a new monitor and start watching for device events.
    ///
    /// Spawns a background thread with its own Tokio runtime for async zbus.
    pub fn start() -> Result<Self, TransportError> {
        let (tx, rx) = mpsc::unbounded_channel();

        let handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build tokio runtime");
            rt.block_on(async {
                if let Err(e) = run_monitor(tx).await {
                    log::error!("BlueZ monitor error: {e}");
                }
            });
        });

        Ok(Self {
            rx: Mutex::new(rx),
            _handle: handle,
        })
    }
}

impl DeviceMonitor for BluezMonitor {
    fn next_event(&self) -> Result<DeviceEvent, TransportError> {
        let mut rx = self
            .rx
            .lock()
            .map_err(|_| TransportError::new("lock poisoned"))?;
        loop {
            match rx.try_recv() {
                Ok(event) => return Ok(event),
                Err(mpsc::error::TryRecvError::Empty) => {
                    drop(rx);
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    rx = self
                        .rx
                        .lock()
                        .map_err(|_| TransportError::new("lock poisoned"))?;
                }
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    return Err(TransportError::ChannelClosed);
                }
            }
        }
    }
}

/// Async monitor implementation using zbus async API.
async fn run_monitor(tx: mpsc::UnboundedSender<DeviceEvent>) -> Result<(), TransportError> {
    log::debug!("Connecting to system D-Bus...");
    let conn = zbus::Connection::system()
        .await
        .map_err(|e| TransportError::with_source("failed to connect to system D-Bus", e))?;
    log::debug!("System D-Bus connected");

    // Check already-connected devices
    check_already_connected_async(&conn, &tx).await;

    // Subscribe to PropertiesChanged on BlueZ
    let rule = "type='signal',sender='org.bluez',interface='org.freedesktop.DBus.Properties',member='PropertiesChanged'";
    conn.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "AddMatch",
        &rule,
    )
    .await
    .map_err(|e| TransportError::with_source("failed to add D-Bus match rule", e))?;

    log::debug!("D-Bus match rule added, listening for PropertiesChanged signals...");

    // Devices confirmed as AirPods (path → address).
    let mut confirmed_devices: HashMap<String, String> = HashMap::new();
    // Devices that are connected but whose UUIDs haven't resolved yet.
    let mut pending_devices: HashSet<String> = HashSet::new();

    // Use the message stream
    let mut stream = zbus::MessageStream::from(&conn);
    use futures::StreamExt;

    while let Some(msg_result) = stream.next().await {
        let msg = match msg_result {
            Ok(m) => m,
            Err(e) => {
                log::warn!("D-Bus message error: {e}");
                continue;
            }
        };

        let header = msg.header();
        let member_name = header.member().map(|m| m.to_string()).unwrap_or_default();
        if member_name != "PropertiesChanged" {
            continue;
        }

        let path = match header.path() {
            Some(p) => p.to_string(),
            None => continue,
        };

        if !path.starts_with("/org/bluez/") || !path.contains("/dev_") {
            continue;
        }

        let body: Result<
            (
                String,
                HashMap<String, zbus::zvariant::OwnedValue>,
                Vec<String>,
            ),
            _,
        > = msg.body().deserialize();
        let (interface, changed, _) = match body {
            Ok(b) => b,
            Err(_) => continue,
        };

        if interface != "org.bluez.Device1" {
            continue;
        }

        let changed_keys: Vec<&String> = changed.keys().collect();
        log::debug!("PropertiesChanged on {path}: {changed_keys:?}");

        // --- Handle Connected property change ---
        if let Some(connected_val) = changed.get("Connected") {
            let connected: bool = <bool>::try_from(connected_val.clone()).unwrap_or(false);

            if connected {
                let address = get_device_property_async(&conn, &path, "Address")
                    .await
                    .unwrap_or_default();
                let name = get_device_property_async(&conn, &path, "Name")
                    .await
                    .unwrap_or_else(|| "Unknown".to_string());

                log::debug!("Device connected: {name} ({address}) at {path}");

                let uuids = get_device_uuids_async(&conn, &path).await;
                log::debug!("  UUIDs (immediate): {uuids:?}");

                if contains_airpods_uuid(&uuids) {
                    log::info!("AirPods connected: {name} ({address})");
                    confirmed_devices.insert(path.clone(), address.clone());
                    pending_devices.remove(&path);
                    let _ = tx.send(DeviceEvent::Connected { address, name });
                } else {
                    // UUIDs not resolved yet — mark as pending and wait for
                    // ServicesResolved / UUIDs property to arrive later.
                    log::debug!(
                        "Device {address} connected but AirPods UUID not found yet, \
                         adding to pending list (will re-check on ServicesResolved/UUIDs change)"
                    );
                    pending_devices.insert(path.clone());
                }
            } else {
                // Device disconnected
                pending_devices.remove(&path);
                if let Some(address) = confirmed_devices.remove(&path) {
                    log::info!("AirPods disconnected: {address}");
                    let _ = tx.send(DeviceEvent::Disconnected { address });
                }
            }
        }

        // --- Handle ServicesResolved / UUIDs change for pending devices ---
        let services_resolved = changed
            .get("ServicesResolved")
            .and_then(|v| <bool>::try_from(v.clone()).ok());
        let uuids_changed = changed.contains_key("UUIDs");

        if (services_resolved == Some(true) || uuids_changed) && pending_devices.contains(&path) {
            let uuids = if uuids_changed {
                // Try to parse UUIDs directly from the signal payload first
                let from_signal = changed
                    .get("UUIDs")
                    .and_then(|v| <Vec<String>>::try_from(v.clone()).ok())
                    .unwrap_or_default();
                if from_signal.is_empty() {
                    // Signal parsing failed; re-read from D-Bus
                    get_device_uuids_async(&conn, &path).await
                } else {
                    from_signal
                }
            } else {
                get_device_uuids_async(&conn, &path).await
            };

            log::debug!("Re-checking pending device {path}, UUIDs: {uuids:?}");

            if contains_airpods_uuid(&uuids) {
                let address = get_device_property_async(&conn, &path, "Address")
                    .await
                    .unwrap_or_default();
                let name = get_device_property_async(&conn, &path, "Name")
                    .await
                    .unwrap_or_else(|| "AirPods".to_string());
                log::info!("AirPods identified after service resolution: {name} ({address})");
                confirmed_devices.insert(path.clone(), address.clone());
                pending_devices.remove(&path);
                let _ = tx.send(DeviceEvent::Connected { address, name });
            }
        }
    }

    Ok(())
}

/// Check for already-connected AirPods at startup.
async fn check_already_connected_async(
    conn: &zbus::Connection,
    tx: &mpsc::UnboundedSender<DeviceEvent>,
) {
    log::debug!("Checking for already-connected AirPods...");

    let reply = match conn
        .call_method(
            Some("org.bluez"),
            "/",
            Some("org.freedesktop.DBus.ObjectManager"),
            "GetManagedObjects",
            &(),
        )
        .await
    {
        Ok(r) => r,
        Err(e) => {
            log::warn!("Failed to get managed objects: {e}");
            return;
        }
    };

    let objects: HashMap<
        zbus::zvariant::OwnedObjectPath,
        HashMap<String, HashMap<String, zbus::zvariant::OwnedValue>>,
    > = match reply.body().deserialize() {
        Ok(o) => o,
        Err(e) => {
            log::warn!("Failed to deserialize managed objects: {e}");
            return;
        }
    };

    let mut device_count = 0u32;
    let mut connected_count = 0u32;

    for (obj_path, interfaces) in &objects {
        if let Some(props) = interfaces.get("org.bluez.Device1") {
            device_count += 1;

            let address = props
                .get("Address")
                .and_then(|v: &zbus::zvariant::OwnedValue| <String>::try_from(v.clone()).ok())
                .unwrap_or_default();
            let name = props
                .get("Name")
                .and_then(|v: &zbus::zvariant::OwnedValue| <String>::try_from(v.clone()).ok())
                .unwrap_or_else(|| "Unknown".to_string());
            let connected = props
                .get("Connected")
                .and_then(|v: &zbus::zvariant::OwnedValue| <bool>::try_from(v.clone()).ok())
                .unwrap_or(false);
            let uuids: Vec<String> = props
                .get("UUIDs")
                .and_then(|v: &zbus::zvariant::OwnedValue| <Vec<String>>::try_from(v.clone()).ok())
                .unwrap_or_default();

            log::debug!(
                "  Device: {name} ({address}) connected={connected} uuids={} path={obj_path}",
                uuids.len(),
            );

            if !connected {
                continue;
            }

            connected_count += 1;

            if uuids.is_empty() {
                log::debug!(
                    "    Skipping {address}: connected but UUIDs empty (SDP not resolved?)"
                );
                continue;
            }

            let is_airpods = contains_airpods_uuid(&uuids);
            log::debug!("    AirPods UUID match: {is_airpods}");

            if !is_airpods {
                continue;
            }

            log::info!("Found already-connected AirPods: {name} ({address})");
            let _ = tx.send(DeviceEvent::Connected { address, name });
        }
    }

    log::debug!(
        "Startup scan complete: {device_count} known device(s), {connected_count} connected"
    );

    if device_count == 0 {
        log::warn!("No Bluetooth devices known to BlueZ at all. Is bluetoothd running?");
    } else if connected_count == 0 {
        log::warn!(
            "No connected devices found. AirPods may need to be paired first.\n\
             \n\
             To pair AirPods with this machine:\n\
             1. Open the AirPods case lid (keep pods inside)\n\
             2. Press and hold the button on the back of the case until the LED flashes white\n\
             3. Run:  bluetoothctl\n\
             4. Type: scan on\n\
             5. Wait for \"AirPods\" to appear, note the MAC address (e.g. AA:BB:CC:DD:EE:FF)\n\
             6. Type: pair AA:BB:CC:DD:EE:FF\n\
             7. Type: trust AA:BB:CC:DD:EE:FF\n\
             8. Type: connect AA:BB:CC:DD:EE:FF\n\
             9. Type: exit\n\
             \n\
             Tip: Run with --scan to detect nearby AirPods via BLE without pairing."
        );
    }
}

async fn get_device_property_async(
    conn: &zbus::Connection,
    path: &str,
    prop: &str,
) -> Option<String> {
    let msg = conn
        .call_method(
            Some("org.bluez"),
            path,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &("org.bluez.Device1", prop),
        )
        .await
        .map_err(|e| {
            log::debug!("Failed to get property {prop} on {path}: {e}");
            e
        })
        .ok()?;
    let val: zbus::zvariant::OwnedValue = msg.body().deserialize().ok()?;
    String::try_from(val).ok()
}

async fn get_device_uuids_async(conn: &zbus::Connection, path: &str) -> Vec<String> {
    let msg = match conn
        .call_method(
            Some("org.bluez"),
            path,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &("org.bluez.Device1", "UUIDs"),
        )
        .await
    {
        Ok(m) => m,
        Err(e) => {
            log::debug!("Failed to get UUIDs for {path}: {e}");
            return Vec::new();
        }
    };
    let val: zbus::zvariant::OwnedValue = match msg.body().deserialize() {
        Ok(v) => v,
        Err(e) => {
            log::debug!("Failed to deserialize UUIDs for {path}: {e}");
            return Vec::new();
        }
    };
    <Vec<String>>::try_from(val).unwrap_or_default()
}
