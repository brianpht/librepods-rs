// LibrePods - AirPods liberated from Apple's ecosystem
// Copyright (C) 2025 LibrePods contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! L2CAP transport implementation using raw AF_BLUETOOTH sockets via `nix`.
//!
//! References:
//! - `main.cpp` `connectToDevice()` — Qt L2CAP socket

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::atomic::{AtomicBool, Ordering};

use librepods_core::protocol::packets;
use librepods_core::transport::{L2capTransport, TransportError};
use nix::libc;

// ---------------------------------------------------------------------------
// Bluetooth socket address (Linux-specific)
// ---------------------------------------------------------------------------

/// `sockaddr_l2` for Bluetooth L2CAP connections.
#[repr(C)]
struct SockAddrL2 {
    l2_family: u16,
    l2_psm: u16,
    l2_bdaddr: [u8; 6],
    l2_cid: u16,
    l2_bdaddr_type: u8,
}

const AF_BLUETOOTH: i32 = 31;
const BTPROTO_L2CAP: i32 = 0;

/// Parse a Bluetooth MAC address string (e.g. "AA:BB:CC:DD:EE:FF") into 6 bytes.
/// BlueZ stores addresses in little-endian order.
fn parse_bdaddr(addr: &str) -> Result<[u8; 6], TransportError> {
    let parts: Vec<&str> = addr.split(':').collect();
    if parts.len() != 6 {
        return Err(TransportError::new(format!(
            "invalid Bluetooth address: {addr}"
        )));
    }
    let mut bytes = [0u8; 6];
    for (i, part) in parts.iter().enumerate() {
        bytes[5 - i] = u8::from_str_radix(part, 16)
            .map_err(|e| TransportError::with_source(format!("bad hex in address: {part}"), e))?;
    }
    Ok(bytes)
}

// ---------------------------------------------------------------------------
// L2CAP socket implementation
// ---------------------------------------------------------------------------

/// Linux L2CAP socket transport using raw `AF_BLUETOOTH` sockets.
pub struct L2capLinux {
    fd: OwnedFd,
    connected: AtomicBool,
}

impl L2capLinux {
    /// Connect to an AirPods device at `address` (e.g. "AA:BB:CC:DD:EE:FF")
    /// on PSM [`packets::AAP_PSM`] (0x1001).
    pub fn connect(address: &str) -> Result<Self, TransportError> {
        Self::connect_psm(address, packets::AAP_PSM)
    }

    /// Connect to a device on a specific PSM.
    pub fn connect_psm(address: &str, psm: u16) -> Result<Self, TransportError> {
        let bdaddr = parse_bdaddr(address)?;

        // Create Bluetooth L2CAP socket using raw libc.
        // We bypass nix::sys::socket::socket() because nix's
        // AddressFamily::from_i32() does not include AF_BLUETOOTH,
        // causing it to silently fall back to AF_UNSPEC and fail.
        let raw_fd = unsafe {
            libc::socket(
                AF_BLUETOOTH,
                libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC,
                BTPROTO_L2CAP,
            )
        };
        if raw_fd < 0 {
            let err = std::io::Error::last_os_error();
            let hint = match err.raw_os_error() {
                Some(libc::EPERM) | Some(libc::EACCES) => {
                    "\nHint: Bluetooth sockets require elevated privileges. Try one of:\n  \
                     • sudo setcap cap_net_raw,cap_net_admin+eip ./target/release/librepods-cli\n  \
                     • Run with sudo"
                }
                Some(libc::EAFNOSUPPORT) => {
                    "\nHint: AF_BLUETOOTH not supported. Is the bluetooth kernel module loaded?\n  \
                     • sudo modprobe bluetooth"
                }
                _ => "",
            };
            return Err(TransportError::with_source(
                format!("failed to create L2CAP socket (AF_BLUETOOTH, SOCK_SEQPACKET, BTPROTO_L2CAP){hint}"),
                err,
            ));
        }

        // Safety: we just created this fd and checked it's valid (>= 0)
        let fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };

        // Build sockaddr_l2
        let addr = SockAddrL2 {
            l2_family: AF_BLUETOOTH as u16,
            l2_psm: psm.to_le(),
            l2_bdaddr: bdaddr,
            l2_cid: 0,
            l2_bdaddr_type: 0, // BDADDR_BREDR
        };

        // Connect
        let addr_ptr = &addr as *const SockAddrL2 as *const libc::sockaddr;
        let addr_len = std::mem::size_of::<SockAddrL2>() as libc::socklen_t;
        let ret = unsafe { libc::connect(fd.as_raw_fd(), addr_ptr, addr_len) };
        if ret < 0 {
            let err = std::io::Error::last_os_error();
            return Err(TransportError::with_source(
                format!("L2CAP connect to {address} PSM 0x{psm:04X} failed"),
                err,
            ));
        }

        log::info!("L2CAP connected to {address} PSM 0x{psm:04X}");

        Ok(Self {
            fd,
            connected: AtomicBool::new(true),
        })
    }
}

impl L2capTransport for L2capLinux {
    fn send(&self, data: &[u8]) -> Result<(), TransportError> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(TransportError::NotConnected);
        }
        let n = nix::unistd::write(&self.fd, data)
            .map_err(|e| TransportError::with_source("L2CAP write failed", e))?;
        if n != data.len() {
            return Err(TransportError::new(format!(
                "short write: {n}/{} bytes",
                data.len()
            )));
        }
        Ok(())
    }

    fn recv(&self, buf: &mut [u8]) -> Result<usize, TransportError> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(TransportError::NotConnected);
        }
        let n = nix::unistd::read(&self.fd, buf)
            .map_err(|e| TransportError::with_source("L2CAP read failed", e))?;
        if n == 0 {
            self.connected.store(false, Ordering::Relaxed);
            return Err(TransportError::ConnectionClosed);
        }
        Ok(n)
    }

    fn close(&self) -> Result<(), TransportError> {
        self.connected.store(false, Ordering::Relaxed);
        // The OwnedFd will close on drop; explicit close is optional.
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }
}
