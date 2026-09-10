//! The vendor firmware command family and the guard that keeps catbus99 out of it.

use catbus99_proto::ota::*;

/// Decoded from Epomaker's own web driver. `GET_DEVICE_INFO` is the anchor: it matches the
/// `AA 10` command we independently reverse-engineered and already implement, which is what
/// establishes that the rest of the table is the same command family.
#[test]
fn the_opcode_table_matches_the_vendor_driver() {
    assert_eq!(GET_DEVICE_INFO, 0x10);
    assert_eq!(BOOT_ANIMATION, 0x40);
    assert_eq!(OTA_GET_DEVICE_SYSTEM_INFO, 0x80);
    assert_eq!(OTA_VERIFY_FIRMWARE_INFO, 0x81);
    assert_eq!(OTA_DEVICE_ENTER_BOOT, 0x82);
    assert_eq!(OTA_SEND_FIRMWARE_INFO, 0x83);
    assert_eq!(OTA_GET_DEVICE_SN, 0x84);
    assert_eq!(OTA_SWITCH_APP_PARTITION, 0x85);
    assert_eq!(OTA_SET_DEVICE_SN, 0x86);
}

#[test]
fn every_firmware_opcode_is_in_the_guarded_domain() {
    for cmd in FIRMWARE_DOMAIN {
        assert!(is_firmware_domain(cmd), "AA {cmd:02X} escapes the guard");
    }
    assert_eq!(FIRMWARE_DOMAIN.len(), 7);
}

/// The guard covers the contiguous range, not just the named opcodes. An unnamed neighbour
/// of ENTER_BOOT is far likelier to be another firmware operation than something harmless.
#[test]
fn the_whole_contiguous_range_is_guarded() {
    for cmd in 0x80u8..=0x86 {
        assert!(is_firmware_domain(cmd), "AA {cmd:02X} is not guarded");
    }
}

/// Commands catbus99 legitimately sends must not be caught by the guard, or the project
/// breaks itself.
#[test]
fn the_commands_we_actually_use_are_not_blocked() {
    for (cmd, what) in [
        (0x10u8, "device info"),
        (0x12, "keymap read base"),
        (0x16, "keymap read fn"),
        (0x18, "config read"),
        (0x22, "keymap write base"),
        (0x26, "keymap write fn"),
        (0x34, "set clock"),
        (0x50, "TFT upload"),
    ] {
        assert!(
            !is_firmware_domain(cmd),
            "AA {cmd:02X} ({what}) wrongly blocked"
        );
    }
}

#[test]
fn opcodes_outside_the_range_are_not_blocked() {
    for cmd in [0x00u8, 0x7F, 0x87, 0xFC, 0xFF] {
        assert!(!is_firmware_domain(cmd), "AA {cmd:02X} wrongly blocked");
    }
}

/// The refusal message names the command, because "refused" without a name is unactionable.
#[test]
fn firmware_commands_have_names_for_error_messages() {
    assert_eq!(
        firmware_command_name(OTA_DEVICE_ENTER_BOOT),
        "OTA_DEVICE_ENTER_BOOT"
    );
    assert_eq!(
        firmware_command_name(OTA_SWITCH_APP_PARTITION),
        "OTA_SWITCH_APP_PARTITION"
    );
    // Unnamed members of the range still guard, and still say something useful.
    assert!(firmware_command_name(0x87).contains("unnamed"));
}

#[test]
fn running_area_decodes_the_vendor_values() {
    assert_eq!(RunningArea::decode(0x10), RunningArea::App);
    assert_eq!(RunningArea::decode(0x30), RunningArea::Boot);
}

/// Our own TH99 Pro capture reports 0x17 here, which the vendor app does not recognise
/// either. Preserved as Unknown rather than forced into App or Boot: guessing which region
/// a keyboard is running from would be a bad thing to be wrong about.
#[test]
fn an_unrecognised_running_area_is_preserved_not_guessed() {
    assert_eq!(RunningArea::decode(0x17), RunningArea::Unknown(0x17));
    for b in [0x00u8, 0x11, 0x2F, 0x31, 0xFF] {
        assert_eq!(RunningArea::decode(b), RunningArea::Unknown(b));
    }
}
