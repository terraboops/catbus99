//! The TH99 Pro device: transport, discovery, and the flash-wear write governor.
//!
//! # Why these live in one crate
//!
//! Every endurance guarantee catbus99 makes depends on *all* panel writes passing through
//! the governor. That must be enforced by the compiler, not by callers remembering to do
//! it — so [`transport::Device::upload_container`] is crate-private and
//! [`governor::Governor::upload_to_panel`] is the only public door.
//!
//! Two other designs were considered and rejected:
//!
//! * **A capability token** (`upload(payload, permit: &WritePermit)`) does not work across
//!   crates: the permit's constructor has to be public for the governor to mint one, so
//!   anyone else can mint one too.
//! * **A Cargo feature** (`unchecked-writes`) does not work either, because features are
//!   *unified* across a build graph — the moment one crate enables it, every crate in the
//!   same build gets it.
//!
//! Rust's privacy is per-crate, so enforcing the invariant means putting the raw write and
//! the policy that guards it in the same crate. Hence this one.
//!
//! For belt and braces, [`transport::Device::write_report`] — the low-level path used for
//! config-channel commands like setting the clock — refuses any report beginning `AA 50`,
//! so a bulk image upload cannot be hand-rolled out of individual report writes either.

pub mod governor;
pub mod paths;
pub mod transport;

pub use governor::{
    default_state_path, hash_payload, Decision, Governor, GovernorConfig, GovernorError, Lane,
    UploadOutcome, WearReport, WearState,
};
pub use transport::{init, probe, Device, HidError, Interface, InterfaceReport, ProbeReport};
