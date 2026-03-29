// LibrePods - AirPods liberated from Apple's ecosystem
// Copyright (C) 2025 LibrePods contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Linux media control with automatic PipeWire (`wpctl`) / PulseAudio (`pactl`)
//! backend detection, and MPRIS playback via `playerctl` or D-Bus.
//!
//! Reference: `mediacontroller.cpp`

use std::process::Command;
use std::sync::Mutex;

use librepods_core::transport::{MediaControl, TransportError};

// ---------------------------------------------------------------------------
// Backend detection
// ---------------------------------------------------------------------------

/// Which audio backend commands are available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AudioBackend {
    /// PipeWire with WirePlumber (`wpctl`)
    WirePlumber,
    /// PulseAudio (`pactl`)
    PulseAudio,
    /// Nothing found
    None,
}

fn detect_backend() -> AudioBackend {
    if which_exists("wpctl") {
        AudioBackend::WirePlumber
    } else if which_exists("pactl") {
        AudioBackend::PulseAudio
    } else {
        AudioBackend::None
    }
}

fn which_exists(cmd: &str) -> bool {
    Command::new("which")
        .arg(cmd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn playerctl_available() -> bool {
    which_exists("playerctl")
}

// ---------------------------------------------------------------------------
// LinuxMediaControl
// ---------------------------------------------------------------------------

/// Linux media controller with automatic PipeWire/PulseAudio backend selection.
pub struct LinuxMediaControl {
    device_mac_underscored: String,
    backend: AudioBackend,
    has_playerctl: bool,
    /// WirePlumber sink ID for the AirPods (cached after first lookup).
    wpctl_sink_id: Mutex<Option<u32>>,
}

impl Default for LinuxMediaControl {
    fn default() -> Self {
        Self::new()
    }
}

impl LinuxMediaControl {
    pub fn new() -> Self {
        let backend = detect_backend();
        let has_playerctl = playerctl_available();

        match backend {
            AudioBackend::WirePlumber => log::info!("Audio backend: PipeWire (wpctl)"),
            AudioBackend::PulseAudio => log::info!("Audio backend: PulseAudio (pactl)"),
            AudioBackend::None => log::warn!(
                "No audio backend found! Install pipewire+wireplumber or pulseaudio-utils."
            ),
        }
        if !has_playerctl {
            log::debug!("playerctl not found — auto play/pause via MPRIS will use D-Bus fallback");
        }

        Self {
            device_mac_underscored: String::new(),
            backend,
            has_playerctl,
            wpctl_sink_id: Mutex::new(None),
        }
    }

    /// Set the MAC address of the connected AirPods (colon-separated).
    pub fn set_device_address(&mut self, mac: &str) {
        self.device_mac_underscored = mac.replace(':', "_");
        *self.wpctl_sink_id.lock().unwrap() = None;
    }

    /// Activate the A2DP sink profile and set AirPods as default output.
    pub fn activate_audio(&self) -> Result<(), TransportError> {
        match self.backend {
            AudioBackend::WirePlumber => self.wpctl_activate(),
            AudioBackend::PulseAudio => self.pactl_activate(),
            AudioBackend::None => Err(TransportError::new(
                "no audio backend (wpctl/pactl) found — install pipewire or pulseaudio-utils",
            )),
        }
    }

    /// Remove the AirPods audio output device.
    pub fn remove_audio_output(&self) -> Result<(), TransportError> {
        match self.backend {
            AudioBackend::WirePlumber => {
                log::debug!("WirePlumber: audio cleanup (PipeWire handles removal automatically)");
                Ok(())
            }
            AudioBackend::PulseAudio => {
                if let Ok(card) = self.pactl_find_card() {
                    let _ = Command::new("pactl")
                        .args(["set-card-profile", &card, "off"])
                        .status();
                }
                Ok(())
            }
            AudioBackend::None => Ok(()),
        }
    }

    // -----------------------------------------------------------------------
    // WirePlumber (PipeWire) backend
    // -----------------------------------------------------------------------

    fn wpctl_activate(&self) -> Result<(), TransportError> {
        let sink_id = self.wpctl_find_sink()?;
        log::info!("WirePlumber: setting AirPods (sink {sink_id}) as default output");
        let status = Command::new("wpctl")
            .args(["set-default", &sink_id.to_string()])
            .status()
            .map_err(|e| TransportError::with_source("failed to run wpctl set-default", e))?;
        if !status.success() {
            return Err(TransportError::new(format!(
                "wpctl set-default {sink_id} failed"
            )));
        }
        Ok(())
    }

    /// Find the WirePlumber sink ID for the AirPods by parsing `wpctl status`.
    fn wpctl_find_sink(&self) -> Result<u32, TransportError> {
        if let Some(id) = *self.wpctl_sink_id.lock().unwrap() {
            return Ok(id);
        }

        let output = Command::new("wpctl")
            .args(["status"])
            .output()
            .map_err(|e| TransportError::with_source("failed to run wpctl status", e))?;
        let stdout = String::from_utf8_lossy(&output.stdout);

        // Parse lines like:  │      99. Brian's AirPods Pro   [vol: 0.40]
        let mac_lower = self.device_mac_underscored.to_ascii_lowercase();
        let mut in_sinks = false;

        for line in stdout.lines() {
            let trimmed = line.trim().trim_start_matches('│').trim();

            if trimmed.starts_with("Sinks:") {
                in_sinks = true;
                continue;
            }
            if in_sinks
                && (trimmed.starts_with("Sources:")
                    || trimmed.starts_with("Streams:")
                    || trimmed.starts_with("Devices:")
                    || (trimmed.ends_with(':') && !trimmed.contains('.')))
            {
                in_sinks = false;
                continue;
            }
            if !in_sinks {
                continue;
            }

            let line_lower = trimmed.to_ascii_lowercase();
            if line_lower.contains(&mac_lower) || line_lower.contains("airpods") {
                let cleaned = trimmed.trim_start_matches('*').trim();
                if let Some(dot_pos) = cleaned.find('.') {
                    if let Ok(id) = cleaned[..dot_pos].trim().parse::<u32>() {
                        *self.wpctl_sink_id.lock().unwrap() = Some(id);
                        return Ok(id);
                    }
                }
            }
        }

        Err(TransportError::new(format!(
            "no WirePlumber sink found for AirPods (MAC: {}). Run 'wpctl status' to check.",
            self.device_mac_underscored
        )))
    }

    fn wpctl_get_default_sink_name(&self) -> Option<String> {
        let output = Command::new("wpctl")
            .args(["inspect", "@DEFAULT_AUDIO_SINK@"])
            .output()
            .ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.contains("node.name") || line.contains("node.description") {
                return Some(line.to_string());
            }
        }
        Some(stdout.lines().next()?.to_string())
    }

    // -----------------------------------------------------------------------
    // PulseAudio backend
    // -----------------------------------------------------------------------

    fn pactl_activate(&self) -> Result<(), TransportError> {
        let card = self.pactl_find_card()?;
        log::info!("PulseAudio: activating A2DP on card {card}");
        let status = Command::new("pactl")
            .args(["set-card-profile", &card, "a2dp-sink"])
            .status()
            .map_err(|e| TransportError::with_source("pactl set-card-profile failed", e))?;
        if !status.success() {
            return Err(TransportError::new(
                "pactl set-card-profile a2dp-sink failed",
            ));
        }

        std::thread::sleep(std::time::Duration::from_millis(500));

        let output = Command::new("pactl")
            .args(["list", "short", "sinks"])
            .output()
            .map_err(|e| TransportError::with_source("pactl list sinks failed", e))?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.contains(&self.device_mac_underscored) {
                let parts: Vec<&str> = line.split('\t').collect();
                if parts.len() >= 2 {
                    log::info!("PulseAudio: setting default sink to {}", parts[1]);
                    let _ = Command::new("pactl")
                        .args(["set-default-sink", parts[1]])
                        .status();
                    break;
                }
            }
        }

        Ok(())
    }

    fn pactl_find_card(&self) -> Result<String, TransportError> {
        let output = Command::new("pactl")
            .args(["list", "short", "cards"])
            .output()
            .map_err(|e| TransportError::with_source("failed to run pactl list cards", e))?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.contains(&self.device_mac_underscored) {
                let parts: Vec<&str> = line.split('\t').collect();
                if parts.len() >= 2 {
                    return Ok(parts[1].to_string());
                }
            }
        }
        Err(TransportError::new(format!(
            "no PulseAudio card found for {}",
            self.device_mac_underscored
        )))
    }
}

// ---------------------------------------------------------------------------
// MediaControl trait implementation
// ---------------------------------------------------------------------------

impl MediaControl for LinuxMediaControl {
    fn play(&self) -> Result<(), TransportError> {
        if self.has_playerctl {
            let status = Command::new("playerctl")
                .arg("play")
                .status()
                .map_err(|e| TransportError::with_source("playerctl play failed", e))?;
            if !status.success() {
                log::warn!("playerctl play returned non-zero");
            }
        } else {
            let _ = Command::new("dbus-send")
                .args([
                    "--type=method_call",
                    "--dest=org.mpris.MediaPlayer2.playerctld",
                    "/org/mpris/MediaPlayer2",
                    "org.mpris.MediaPlayer2.Player.Play",
                ])
                .status();
        }
        Ok(())
    }

    fn pause(&self) -> Result<(), TransportError> {
        if self.has_playerctl {
            let status = Command::new("playerctl")
                .arg("pause")
                .status()
                .map_err(|e| TransportError::with_source("playerctl pause failed", e))?;
            if !status.success() {
                log::warn!("playerctl pause returned non-zero");
            }
        } else {
            let _ = Command::new("dbus-send")
                .args([
                    "--type=method_call",
                    "--dest=org.mpris.MediaPlayer2.playerctld",
                    "/org/mpris/MediaPlayer2",
                    "org.mpris.MediaPlayer2.Player.Pause",
                ])
                .status();
        }
        Ok(())
    }

    fn set_volume(&self, percent: u8) -> Result<(), TransportError> {
        match self.backend {
            AudioBackend::WirePlumber => {
                let frac = format!("{:.2}", percent as f64 / 100.0);
                let status = Command::new("wpctl")
                    .args(["set-volume", "@DEFAULT_AUDIO_SINK@", &frac])
                    .status()
                    .map_err(|e| TransportError::with_source("wpctl set-volume failed", e))?;
                if !status.success() {
                    log::warn!("wpctl set-volume returned non-zero");
                }
            }
            AudioBackend::PulseAudio => {
                let status = Command::new("pactl")
                    .args(["set-sink-volume", "@DEFAULT_SINK@", &format!("{percent}%")])
                    .status()
                    .map_err(|e| TransportError::with_source("pactl set-sink-volume failed", e))?;
                if !status.success() {
                    log::warn!("pactl set-sink-volume returned non-zero");
                }
            }
            AudioBackend::None => {}
        }
        Ok(())
    }

    fn get_volume(&self) -> Result<u8, TransportError> {
        match self.backend {
            AudioBackend::WirePlumber => {
                let output = Command::new("wpctl")
                    .args(["get-volume", "@DEFAULT_AUDIO_SINK@"])
                    .output()
                    .map_err(|e| TransportError::with_source("wpctl get-volume failed", e))?;
                let stdout = String::from_utf8_lossy(&output.stdout);
                // "Volume: 0.40" or "Volume: 0.40 [MUTED]"
                if let Some(rest) = stdout.strip_prefix("Volume:") {
                    let num_str = rest.split_whitespace().next().unwrap_or("0");
                    if let Ok(frac) = num_str.parse::<f64>() {
                        return Ok((frac * 100.0).round() as u8);
                    }
                }
                Err(TransportError::new("failed to parse wpctl volume"))
            }
            AudioBackend::PulseAudio => {
                let output = Command::new("pactl")
                    .args(["get-sink-volume", "@DEFAULT_SINK@"])
                    .output()
                    .map_err(|e| TransportError::with_source("pactl get-sink-volume failed", e))?;
                let stdout = String::from_utf8_lossy(&output.stdout);
                for part in stdout.split('/') {
                    let trimmed = part.trim();
                    if let Some(pct) = trimmed.strip_suffix('%') {
                        if let Ok(val) = pct.trim().parse::<u8>() {
                            return Ok(val);
                        }
                    }
                }
                Err(TransportError::new("failed to parse pactl volume"))
            }
            AudioBackend::None => Err(TransportError::new("no audio backend")),
        }
    }

    fn is_airpods_active_sink(&self) -> bool {
        if self.device_mac_underscored.is_empty() {
            return false;
        }
        match self.backend {
            AudioBackend::WirePlumber => self
                .wpctl_get_default_sink_name()
                .map(|s| {
                    let s_lower = s.to_ascii_lowercase();
                    s_lower.contains(&self.device_mac_underscored.to_ascii_lowercase())
                        || s_lower.contains("airpods")
                })
                .unwrap_or(false),
            AudioBackend::PulseAudio => Command::new("pactl")
                .args(["get-default-sink"])
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).contains(&self.device_mac_underscored))
                .unwrap_or(false),
            AudioBackend::None => false,
        }
    }
}
