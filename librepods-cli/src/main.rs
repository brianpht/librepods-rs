// LibrePods - AirPods liberated from Apple's ecosystem
// Copyright (C) 2025 LibrePods contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! CLI daemon for LibrePods.
//!
//! Wires the core library and Linux platform implementations together into a
//! headless daemon that:
//! 1. Watches for AirPods connection via BlueZ D-Bus
//! 2. Connects via L2CAP, performs handshake
//! 3. Parses incoming packets and logs device state
//! 4. Optionally controls media via MPRIS on ear detection events

use std::time::Duration;

use clap::Parser;
use librepods_core::device::ble_advert;
use librepods_core::device::state::{model_from_number, DeviceState};
use librepods_core::protocol::{packets, parser};
use librepods_core::transport::{BleScanner, DeviceMonitor, L2capTransport, MediaControl};
use librepods_linux::ble::BtleplugScanner;
use librepods_linux::bluez_monitor::BluezMonitor;
use librepods_linux::l2cap::L2capLinux;
use librepods_linux::media::LinuxMediaControl;

#[derive(Parser)]
#[command(name = "librepods", about = "LibrePods CLI daemon")]
struct Cli {
    /// Bluetooth address to connect to directly (e.g. "AA:BB:CC:DD:EE:FF").
    /// If not given, will auto-detect via BlueZ.
    #[arg(short, long)]
    address: Option<String>,

    /// Maximum connection retry attempts.
    #[arg(long, default_value = "3")]
    retries: u32,

    /// Scan for nearby AirPods via BLE advertisements without connecting.
    /// Useful for finding AirPods that haven't been paired yet.
    #[arg(short, long)]
    scan: bool,

    /// Disable media control (no auto play/pause, no A2DP activation).
    #[arg(long)]
    no_media: bool,
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let cli = Cli::parse();

    if cli.scan {
        run_ble_scan();
        return;
    }

    let address = if let Some(addr) = cli.address {
        log::info!("Using provided address: {addr}");
        addr
    } else {
        log::info!("Watching for AirPods via BlueZ...");
        let monitor = match BluezMonitor::start() {
            Ok(m) => m,
            Err(e) => {
                log::error!("Failed to start BlueZ monitor: {e}");
                std::process::exit(1);
            }
        };

        loop {
            match monitor.next_event() {
                Ok(librepods_core::transport::DeviceEvent::Connected { address, name }) => {
                    log::info!("AirPods detected: {name} ({address})");
                    break address;
                }
                Ok(librepods_core::transport::DeviceEvent::Disconnected { address }) => {
                    log::info!("Device disconnected: {address}");
                }
                Err(e) => {
                    log::error!("Monitor error: {e}");
                    std::process::exit(1);
                }
            }
        }
    };

    // Connect with retries
    let transport = connect_with_retries(&address, cli.retries);
    let transport = match transport {
        Some(t) => t,
        None => {
            log::error!("Failed to connect after {} attempts", cli.retries);
            std::process::exit(1);
        }
    };

    // Run the handshake state machine
    let mut state = DeviceState {
        bluetooth_address: address.clone(),
        ..DeviceState::default()
    };

    let media = if cli.no_media {
        log::info!("Media control disabled (--no-media)");
        None
    } else {
        let mut mc = LinuxMediaControl::new();
        mc.set_device_address(&address);
        Some(mc)
    };

    if let Err(e) = run_connection(&transport, &mut state, media.as_ref()) {
        log::error!("Connection ended: {e}");
    }

    log::info!("Disconnected. Final state: {state:#?}");
}

/// BLE scan mode: discover nearby AirPods via BLE advertisements and print their info.
fn run_ble_scan() {
    log::info!("Scanning for nearby AirPods via BLE...");
    log::info!("Open the AirPods case lid to make them discoverable. Press Ctrl+C to stop.\n");

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    let scanner = match rt.block_on(BtleplugScanner::start()) {
        Ok(s) => s,
        Err(e) => {
            log::error!("Failed to start BLE scanner: {e}");
            log::error!("Make sure Bluetooth is enabled and you have permission to scan.");
            log::error!("On Linux, you may need to run: sudo setcap cap_net_raw+eip <binary>");
            std::process::exit(1);
        }
    };

    // Track seen addresses to avoid repeating the same AirPods
    let mut seen = std::collections::HashSet::new();

    loop {
        match scanner.next_advertisement() {
            Ok(advert) => {
                if let Some(parsed) = ble_advert::parse_advertisement(&advert.manufacturer_data) {
                    let key = (advert.address.clone(), parsed.model_id);
                    let is_new = seen.insert(key);

                    if is_new {
                        let rssi_str = advert
                            .rssi
                            .map(|r| format!("{r} dBm"))
                            .unwrap_or_else(|| "unknown".to_string());

                        println!("┌─────────────────────────────────────────────");
                        println!("│ 🎧 AirPods found!");
                        println!("│ Model:       {:?}", parsed.model);
                        println!(
                            "│ BLE address: {} (random — NOT the pairing address)",
                            advert.address
                        );
                        println!("│ RSSI:        {rssi_str}");
                        println!(
                            "│ Paired:      {}",
                            if parsed.paired {
                                "yes (to another device)"
                            } else {
                                "no"
                            }
                        );

                        if let Some(l) = parsed.left_battery {
                            println!(
                                "│ Left:        {l}%{}",
                                if parsed.left_charging { " ⚡" } else { "" }
                            );
                        }
                        if let Some(r) = parsed.right_battery {
                            println!(
                                "│ Right:       {r}%{}",
                                if parsed.right_charging { " ⚡" } else { "" }
                            );
                        }
                        if let Some(c) = parsed.case_battery {
                            println!(
                                "│ Case:        {c}%{}",
                                if parsed.case_charging { " ⚡" } else { "" }
                            );
                        }
                        println!("│ Lid:         {:?}", parsed.lid_state);
                        println!("│ Connection:  {:?}", parsed.connection_state);
                        println!("├─────────────────────────────────────────────");
                        println!(
                            "│ ⚠ BLE address is random and changes. To pair, use bluetoothctl:"
                        );
                        println!("│");
                        println!("│   1. Hold the case button until LED flashes white");
                        println!("│   2. bluetoothctl");
                        println!("│   3. scan on");
                        println!("│   4. Look for your AirPods name and note the address");
                        println!("│   5. pair <address>");
                        println!("│   6. trust <address>");
                        println!("│   7. connect <address>");
                        println!("│");
                        println!("│ After pairing, run: librepods-cli (without --scan)");
                        println!("└─────────────────────────────────────────────\n");
                    }
                }
            }
            Err(e) => {
                log::error!("BLE scan error: {e}");
                break;
            }
        }
    }
}

fn connect_with_retries(address: &str, max_retries: u32) -> Option<L2capLinux> {
    for attempt in 1..=max_retries {
        log::info!("Connection attempt {attempt}/{max_retries} to {address}");
        match L2capLinux::connect(address) {
            Ok(t) => return Some(t),
            Err(e) => {
                log::warn!("Attempt {attempt} failed: {e}");
                if attempt < max_retries {
                    std::thread::sleep(Duration::from_millis(1500));
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Ear-detection → media control helper
// ---------------------------------------------------------------------------

/// Manages automatic A2DP activation and play/pause based on ear detection.
///
/// Keeps internal state about whether audio has been activated and whether
/// we auto-paused so we can resume when a pod goes back in ear.
struct EarMediaController<'a> {
    media: &'a LinuxMediaControl,
    audio_activated: bool,
    paused_by_us: bool,
}

impl<'a> EarMediaController<'a> {
    fn new(media: &'a LinuxMediaControl) -> Self {
        Self {
            media,
            audio_activated: false,
            paused_by_us: false,
        }
    }

    /// Try to activate A2DP audio output (idempotent after first success).
    fn try_activate(&mut self) {
        if self.audio_activated {
            return;
        }
        log::info!("Activating A2DP audio output for AirPods...");
        match self.media.activate_audio() {
            Ok(()) => {
                self.audio_activated = true;
                log::info!("🔊 Audio routed to AirPods — you can listen now!");
            }
            Err(e) => {
                log::warn!("Failed to activate audio: {e}");
                log::warn!("You may need to manually set AirPods as audio output.");
            }
        }
    }

    /// Called when metadata is first received — activate if pods are in ear.
    fn on_metadata(&mut self, state: &DeviceState) {
        if state.any_pod_in_ear() {
            self.try_activate();
        } else {
            log::info!("AirPods not in ear yet — will activate audio when worn");
        }
    }

    /// Called on ear detection changes — activate, resume, or pause.
    fn on_ear_detection(&mut self, primary_in_ear: bool, secondary_in_ear: bool) {
        if primary_in_ear || secondary_in_ear {
            // At least one pod in ear
            self.try_activate();
            if self.paused_by_us && self.media.is_airpods_active_sink() {
                log::info!("▶ Resuming playback (pod back in ear)");
                let _ = self.media.play();
                self.paused_by_us = false;
            }
        } else {
            // Both pods out of ear
            if self.audio_activated && self.media.is_airpods_active_sink() {
                log::info!("⏸ Pausing playback (both pods removed)");
                if self.media.pause().is_ok() {
                    self.paused_by_us = true;
                }
            }
        }
    }

    /// Cleanup on disconnect — remove AirPods audio output.
    fn cleanup(&self) {
        if self.audio_activated {
            log::info!("Removing AirPods audio output...");
            let _ = self.media.remove_audio_output();
        }
    }
}

// ---------------------------------------------------------------------------
// Connection state machine
// ---------------------------------------------------------------------------

fn run_connection(
    transport: &L2capLinux,
    state: &mut DeviceState,
    media: Option<&LinuxMediaControl>,
) -> Result<(), Box<dyn std::error::Error>> {
    // --- Handshake sequence ---

    // 1. Send handshake
    log::info!("Sending handshake...");
    transport.send(&packets::HANDSHAKE)?;

    // 2. Wait for handshake ACK, then send SET_SPECIFIC_FEATURES
    let mut buf = [0u8; 1024];
    let n = transport.recv(&mut buf)?;
    let pkt = &buf[..n];
    log::debug!("Received {} bytes: {:02X?}", n, pkt);

    match parser::parse(pkt)? {
        parser::ParsedPacket::HandshakeAck => {
            log::info!("Handshake ACK received, sending SET_SPECIFIC_FEATURES");
            transport.send(&packets::SET_SPECIFIC_FEATURES)?;
        }
        other => {
            log::warn!("Expected HandshakeAck, got: {other:?}");
            // Try sending features anyway
            transport.send(&packets::SET_SPECIFIC_FEATURES)?;
        }
    }

    // 3. Wait for features ACK, then send REQUEST_NOTIFICATIONS
    let n = transport.recv(&mut buf)?;
    let pkt = &buf[..n];
    log::debug!("Received {} bytes: {:02X?}", n, pkt);

    match parser::parse(pkt)? {
        parser::ParsedPacket::FeaturesAck => {
            log::info!("Features ACK received, sending REQUEST_NOTIFICATIONS");
            transport.send(&packets::REQUEST_NOTIFICATIONS)?;
        }
        other => {
            log::warn!("Expected FeaturesAck, got: {other:?}");
            transport.send(&packets::REQUEST_NOTIFICATIONS)?;
        }
    }

    log::info!("Handshake complete. Entering receive loop...");

    let mut ear_mc = media.map(EarMediaController::new);

    // --- Main receive loop ---
    loop {
        let n = match transport.recv(&mut buf) {
            Ok(n) => n,
            Err(e) => {
                log::error!("Receive error: {e}");
                break;
            }
        };

        let pkt = &buf[..n];

        // Skip logging head tracking data (very frequent)
        if !packets::is_head_tracking_data(pkt) {
            log::debug!(
                "Received {} bytes: {}",
                n,
                pkt.iter()
                    .map(|b| format!("{b:02X}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
        }

        match parser::parse(pkt) {
            Ok(parsed) => {
                let is_first_metadata = matches!(&parsed, parser::ParsedPacket::Metadata { .. })
                    && ear_mc.as_ref().is_some_and(|emc| !emc.audio_activated);

                let ear_event = match &parsed {
                    parser::ParsedPacket::EarDetection {
                        primary_in_ear,
                        secondary_in_ear,
                    } => Some((*primary_in_ear, *secondary_in_ear)),
                    _ => None,
                };

                apply_to_state(state, parsed);

                if is_first_metadata {
                    if let Some(emc) = ear_mc.as_mut() {
                        emc.on_metadata(state);
                    }
                }

                if let Some((primary, secondary)) = ear_event {
                    if let Some(emc) = ear_mc.as_mut() {
                        emc.on_ear_detection(primary, secondary);
                    }
                }
            }
            Err(e) => {
                log::warn!("Parse error: {e}");
            }
        }
    }

    // Cleanup
    if let Some(emc) = ear_mc.as_ref() {
        emc.cleanup();
    }

    Ok(())
}

fn apply_to_state(state: &mut DeviceState, packet: parser::ParsedPacket) {
    match packet {
        parser::ParsedPacket::Battery(info) => {
            log::info!(
                "Battery — L:{}% R:{}% Case:{}%",
                info.left.level,
                info.right.level,
                info.case_.level,
            );
            state.battery = info;
        }

        parser::ParsedPacket::EarDetection {
            primary_in_ear,
            secondary_in_ear,
        } => {
            log::info!(
                "Ear detection — primary:{} secondary:{}",
                if primary_in_ear { "in" } else { "out" },
                if secondary_in_ear { "in" } else { "out" },
            );
            state.primary_in_ear = primary_in_ear;
            state.secondary_in_ear = secondary_in_ear;
        }

        parser::ParsedPacket::NoiseControl(mode) => {
            log::info!("Noise control mode: {mode:?}");
            state.noise_control = mode;
        }

        parser::ParsedPacket::ConversationalAwareness { status } => {
            log::info!("Conversational awareness status: 0x{status:02X}");
        }

        parser::ParsedPacket::ControlCommand { id, data } => {
            log::info!("Control command: {id:?} data={data:02X?}");

            // Update specific state fields based on the command
            use librepods_core::protocol::control_command::ControlCommandId;
            match id {
                ControlCommandId::ConversationDetectConfig => {
                    state.conversational_awareness = data[0] == 0x01;
                }
                ControlCommandId::OneBudAncMode => {
                    state.one_bud_anc = data[0] == 0x01;
                }
                _ => {}
            }
        }

        parser::ParsedPacket::Metadata {
            name,
            model_number,
            manufacturer,
        } => {
            log::info!("Metadata — name:{name} model:{model_number} mfr:{manufacturer}");
            state.device_name = name;
            state.model = model_from_number(&model_number);
            state.model_number = model_number;
            state.manufacturer = manufacturer;
        }

        parser::ParsedPacket::HeadTracking(_) => {
            // Too frequent to log
        }

        parser::ParsedPacket::ProximityKeys(keys) => {
            for key in &keys {
                log::info!(
                    "Proximity key: {:?} ({} bytes)",
                    key.key_type,
                    key.data.len()
                );
            }
            for key in keys {
                match key.key_type {
                    parser::ProximityKeyType::Irk => {
                        if key.data.len() == 16 {
                            let mut arr = [0u8; 16];
                            arr.copy_from_slice(&key.data);
                            state.magic_acc_irk = Some(arr);
                        }
                    }
                    parser::ProximityKeyType::EncKey => {
                        if key.data.len() == 16 {
                            let mut arr = [0u8; 16];
                            arr.copy_from_slice(&key.data);
                            state.magic_acc_enc_key = Some(arr);
                        }
                    }
                }
            }
        }

        parser::ParsedPacket::MagicCloudKeys { irk, enc_key } => {
            log::info!("Magic cloud keys received (IRK + EncKey)");
            state.magic_acc_irk = Some(irk);
            state.magic_acc_enc_key = Some(enc_key);
        }

        parser::ParsedPacket::HandshakeAck | parser::ParsedPacket::FeaturesAck => {
            log::debug!("Late handshake/features ACK (ignored)");
        }

        parser::ParsedPacket::Unknown(data) => {
            log::debug!(
                "Unknown packet ({} bytes): {}",
                data.len(),
                data.iter()
                    .map(|b| format!("{b:02X}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
        }
    }
}
