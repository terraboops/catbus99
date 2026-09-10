//! The vendor firmware (OTA) command family, and why catbus99 refuses to send it.
//!
//! Decoded from Epomaker's own web driver at `epomaker.driveall.cn`, which carries a
//! complete over-the-air firmware implementation. These share the `AA <cmd>` config-channel
//! framing with the commands catbus99 does use, so nothing in the transport would stop us
//! sending them by accident.
//!
//! # catbus99 must never send these
//!
//! Every one of them operates on firmware rather than on pixels, keymaps or the clock.
//! [`OTA_DEVICE_ENTER_BOOT`] in particular reboots the keyboard into its bootloader, where
//! the application firmware is not running: the keyboard stops being a keyboard until
//! something completes an upgrade or switches the partition back. That is not a failure
//! mode this project should be able to reach by accident, so
//! [`is_firmware_domain`] is enforced in the transport rather than left to callers.
//!
//! Documented here because knowing the opcodes is genuinely useful — it establishes that a
//! software path into the bootloader exists, which matters a great deal for any future
//! firmware work — and because a guard against a range you cannot name is hard to review.

/// Read device info: bootloader version, app version, VID, PID, running area.
///
/// The one member of this table catbus99 *does* use; it is read-only and carries no
/// firmware payload. Confirmed against our own captures: the reply decodes to
/// manufacturer `0x0C45` and product `0x800A` exactly.
pub const GET_DEVICE_INFO: u8 = 0x10;

/// Boot animation control. Not firmware, but adjacent and undocumented; left alone.
pub const BOOT_ANIMATION: u8 = 0x40;

/// Read the device's system information block.
pub const OTA_GET_DEVICE_SYSTEM_INFO: u8 = 0x80;
/// Verify a firmware image the device has been sent.
pub const OTA_VERIFY_FIRMWARE_INFO: u8 = 0x81;
/// **Reboot the keyboard into its bootloader.** The application stops running.
pub const OTA_DEVICE_ENTER_BOOT: u8 = 0x82;
/// Send firmware metadata ahead of an image.
pub const OTA_SEND_FIRMWARE_INFO: u8 = 0x83;
/// Read the device serial number.
pub const OTA_GET_DEVICE_SN: u8 = 0x84;
/// Switch which application partition the device runs from.
pub const OTA_SWITCH_APP_PARTITION: u8 = 0x85;
/// Write the device serial number.
pub const OTA_SET_DEVICE_SN: u8 = 0x86;

/// Every opcode in the firmware domain, for guards and diagnostics.
pub const FIRMWARE_DOMAIN: [u8; 7] = [
    OTA_GET_DEVICE_SYSTEM_INFO,
    OTA_VERIFY_FIRMWARE_INFO,
    OTA_DEVICE_ENTER_BOOT,
    OTA_SEND_FIRMWARE_INFO,
    OTA_GET_DEVICE_SN,
    OTA_SWITCH_APP_PARTITION,
    OTA_SET_DEVICE_SN,
];

/// True for any command that operates on firmware rather than on device content.
///
/// Deliberately covers the whole `0x80..=0x86` range rather than only the opcodes we have
/// names for: the range is contiguous in the vendor's own table, and an unnamed neighbour
/// is far more likely to be another firmware operation than something safe.
pub fn is_firmware_domain(command: u8) -> bool {
    (0x80..=0x86).contains(&command)
}

/// A human-readable name for a firmware-domain opcode, for error messages.
pub fn firmware_command_name(command: u8) -> &'static str {
    match command {
        OTA_GET_DEVICE_SYSTEM_INFO => "OTA_GET_DEVICE_SYSTEM_INFO",
        OTA_VERIFY_FIRMWARE_INFO => "OTA_VERIFY_FIRMWARE_INFO",
        OTA_DEVICE_ENTER_BOOT => "OTA_DEVICE_ENTER_BOOT",
        OTA_SEND_FIRMWARE_INFO => "OTA_SEND_FIRMWARE_INFO",
        OTA_GET_DEVICE_SN => "OTA_GET_DEVICE_SN",
        OTA_SWITCH_APP_PARTITION => "OTA_SWITCH_APP_PARTITION",
        OTA_SET_DEVICE_SN => "OTA_SET_DEVICE_SN",
        _ => "unnamed firmware-domain command",
    }
}

/// Which region the device reports itself running from, per `GET_DEVICE_INFO`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunningArea {
    /// Normal operation: the application firmware is running.
    App,
    /// The bootloader is running; the keyboard is not functioning as a keyboard.
    Boot,
    /// A value the vendor app does not recognise either.
    ///
    /// Our own TH99 Pro capture reports `0x17` here, which is neither of the vendor's two
    /// values — so on this model the byte may carry something else entirely. Reported
    /// rather than guessed.
    Unknown(u8),
}

impl RunningArea {
    pub fn decode(byte: u8) -> Self {
        match byte {
            0x10 => RunningArea::App,
            0x30 => RunningArea::Boot,
            other => RunningArea::Unknown(other),
        }
    }
}
