// LibrePods - AirPods liberated from Apple's ecosystem
// Copyright (C) 2025 LibrePods contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! BLE scanner using `btleplug` (cross-platform).
//!
//! Scans for Apple manufacturer data (company ID `0x004C`) and yields raw
//! advertisement bytes for parsing by `librepods_core::device::ble_advert`.

use std::sync::Mutex;
use std::time::Duration;

use btleplug::api::{Central, Manager as _, Peripheral, ScanFilter};
use btleplug::platform::Manager;
use librepods_core::transport::{BleScanner, RawBleAdvertisement, TransportError};
use tokio::sync::mpsc;

/// Apple Bluetooth company ID.
const APPLE_COMPANY_ID: u16 = 0x004C;

/// BLE scanner backed by `btleplug`.
pub struct BtleplugScanner {
    rx: Mutex<mpsc::UnboundedReceiver<RawBleAdvertisement>>,
    _handle: tokio::task::JoinHandle<()>,
}

impl BtleplugScanner {
    /// Start scanning. Must be called from within a Tokio runtime.
    pub async fn start() -> Result<Self, TransportError> {
        let manager = Manager::new()
            .await
            .map_err(|e| TransportError::with_source("btleplug Manager::new failed", e))?;

        let adapters = manager
            .adapters()
            .await
            .map_err(|e| TransportError::with_source("failed to get BLE adapters", e))?;

        let adapter = adapters
            .into_iter()
            .next()
            .ok_or_else(|| TransportError::new("no BLE adapter found"))?;

        adapter
            .start_scan(ScanFilter::default())
            .await
            .map_err(|e| TransportError::with_source("BLE scan start failed", e))?;

        let (tx, rx) = mpsc::unbounded_channel();

        let handle = tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(500)).await;

                let peripherals = match adapter.peripherals().await {
                    Ok(p) => p,
                    Err(e) => {
                        log::warn!("failed to list peripherals: {e}");
                        continue;
                    }
                };

                for peripheral in peripherals {
                    let props = match peripheral.properties().await {
                        Ok(Some(p)) => p,
                        _ => continue,
                    };

                    if let Some(data) = props.manufacturer_data.get(&APPLE_COMPANY_ID) {
                        // Only interested in Proximity Pairing Messages (type 0x07)
                        if data.first() != Some(&0x07) {
                            continue;
                        }

                        let address = props.address.to_string();
                        let rssi = props.rssi;

                        let advert = RawBleAdvertisement {
                            address,
                            manufacturer_data: data.clone(),
                            rssi,
                        };

                        if tx.send(advert).is_err() {
                            return; // receiver dropped
                        }
                    }
                }
            }
        });

        Ok(Self {
            rx: Mutex::new(rx),
            _handle: handle,
        })
    }
}

impl BleScanner for BtleplugScanner {
    fn next_advertisement(&self) -> Result<RawBleAdvertisement, TransportError> {
        loop {
            let mut rx = self
                .rx
                .lock()
                .map_err(|_| TransportError::new("lock poisoned"))?;
            match rx.try_recv() {
                Ok(ad) => return Ok(ad),
                Err(mpsc::error::TryRecvError::Empty) => {
                    drop(rx);
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    return Err(TransportError::ChannelClosed);
                }
            }
        }
    }
}
