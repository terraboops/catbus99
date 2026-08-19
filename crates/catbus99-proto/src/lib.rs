//! Wire-format encoder/decoder for the Epomaker TH99 Pro's USB-HID screen protocol.
//!
//! This crate is **pure**: it performs no I/O and touches no device. Everything about
//! the correctness of the wire format is therefore testable in CI, on any machine,
//! without a keyboard attached -- which matters a great deal for a protocol whose only
//! other verification channel is looking at the screen with your eyes.
//!
//! # Channels
//!
//! * [`clock`] -- the `MI_02` config channel (64-byte `AA <cmd>` packets)
//! * [`container`] / [`report`] -- the `MI_03` TFT channel (4104-byte `AA 50` reports)
//! * [`wear`] -- flash-endurance arithmetic
//!
//! # Protocol provenance
//!
//! Every byte layout here was captured from live WebHID traffic of Epomaker's own driver
//! and confirmed against the panel. See `docs/PROTOCOL.md`.

pub mod clock;
pub mod container;
pub mod error;
pub mod keymap;
pub mod report;
pub mod wear;

pub use error::ProtoError;

/// USB vendor ID of the wired TH99 Pro composite device.
pub const VID: u16 = 0x0C45;

/// USB product ID of the wired TH99 Pro composite device.
///
/// Over the 2.4 GHz receiver the keyboard enumerates as a *different* device
/// (`0C45:FEFE`) that exposes neither the config nor the TFT interface, which is why
/// catbus99 requires a wired connection.
pub const PID: u16 = 0x800A;

/// USB interface number of the config channel (Windows calls this `MI_02`).
pub const IFACE_CONFIG: i32 = 2;

/// USB interface number of the TFT channel (Windows calls this `MI_03`).
pub const IFACE_TFT: i32 = 3;
