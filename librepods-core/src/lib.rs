// LibrePods - AirPods liberated from Apple's ecosystem
// Copyright (C) 2025 LibrePods contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Cross-platform core library for LibrePods.
//!
//! This crate contains pure-Rust implementations of:
//! - AAP (Apple Accessory Protocol) packet construction and parsing
//! - Device state management (battery, noise control, ear detection, etc.)
//! - BLE advertisement data decoding
//! - Cross-device sync protocol
//!
//! **No platform-specific Bluetooth or OS dependencies.** All I/O is abstracted
//! behind traits in [`transport`] so this crate compiles for any target.

pub mod crossdevice;
pub mod device;
pub mod protocol;
pub mod transport;
