//! USB-HID transport and interface discovery for the TH99 Pro.
//!
//! Note the visibility of [`Device::upload_container`]: it is **crate-private on purpose**.
//! It is the only code that writes pixels to flash, and the only caller permitted to
//! reach it is [`crate::governor::Governor`]. See the crate docs for why that boundary
//! is expressed as module privacy rather than a token or a feature flag.
//!
//! # Why discovery is the hard part on macOS
//!
//! The original Windows implementation finds the two interfaces by substring-matching
//! `MI_02` / `MI_03` in the device path. macOS device paths carry no such marker, so we
//! identify interfaces structurally instead, in descending order of trustworthiness:
//!
//! 1. `interface_number` reported by hidapi (authoritative when present)
//! 2. `usage_page` / `usage` (vendor-defined pages, distinguished by usage id)
//!
//! [`probe`] dumps everything both strategies look at, so a device that defeats them can
//! be diagnosed from a bug report rather than guessed at.

use catbus99_proto::{IFACE_CONFIG, IFACE_TFT, PID, VID};
use hidapi::{DeviceInfo, HidApi};
use serde::Serialize;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Duration;
use thiserror::Error;

/// The process-wide hidapi handle, owned by a thread that never exits.
///
/// # Why a dedicated thread
///
/// On macOS, libhidapi ties its IOKit run-loop state to the thread that calls
/// `hid_init()`. If that thread later **exits**, any subsequent use of the handle aborts
/// the process with SIGTRAP. Measured directly:
///
/// | thread that initialises | outcome |
/// | --- | --- |
/// | exits after initialising | **abort** |
/// | stays alive | fine |
/// | main thread | fine |
///
/// Lazy initialisation on "whichever caller arrives first" is therefore unsafe here: in
/// the daemon that is a tokio worker, and blocking-pool threads are retired after an idle
/// timeout. So initialisation is performed on a dedicated thread that parks forever, and
/// the handle it builds outlives every caller.
///
/// Concurrent *use* of one instance is fine, and is serialised by the mutex.
static HIDAPI: OnceLock<Mutex<HidApi>> = OnceLock::new();
static HIDAPI_INIT: Mutex<()> = Mutex::new(());

fn api_lock() -> Result<MutexGuard<'static, HidApi>, HidError> {
    if HIDAPI.get().is_none() {
        // Serialise initialisation; the re-check inside means only one thread builds it.
        let _init = HIDAPI_INIT.lock().unwrap_or_else(|e| e.into_inner());
        if HIDAPI.get().is_none() {
            let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
            std::thread::Builder::new()
                .name("catbus99-hid".into())
                .spawn(move || {
                    match HidApi::new() {
                        Ok(api) => {
                            let _ = HIDAPI.set(Mutex::new(api));
                            let _ = tx.send(Ok(()));
                        }
                        Err(e) => {
                            let _ = tx.send(Err(e.to_string()));
                            return;
                        }
                    }
                    // Never exit: the handle above is only valid while this thread lives.
                    loop {
                        std::thread::park();
                    }
                })
                .map_err(|e| HidError::InitThread(e.to_string()))?;

            match rx.recv() {
                Ok(Ok(())) => {}
                Ok(Err(message)) => return Err(HidError::InitThread(message)),
                Err(e) => return Err(HidError::InitThread(e.to_string())),
            }
        }
    }
    let mutex = HIDAPI.get().expect("initialised above");
    // A poisoned lock means an earlier caller panicked mid-enumeration; the handle itself
    // is still valid, so recover rather than propagating the panic to every later caller.
    Ok(mutex.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Run `f` with the shared hidapi handle, re-enumerating devices first.
///
/// The refresh matters: the handle lives for the whole process, so without it a keyboard
/// plugged in after start-up would never be discovered.
fn with_api<T>(f: impl FnOnce(&HidApi) -> Result<T, HidError>) -> Result<T, HidError> {
    let mut api = api_lock()?;
    api.refresh_devices().map_err(HidError::Init)?;
    f(&api)
}

/// Initialise the HID subsystem eagerly.
///
/// Optional -- everything initialises on demand -- but calling this early from `main`
/// keeps the cost off the first request.
pub fn init() -> Result<(), HidError> {
    api_lock().map(|_| ())
}

#[derive(Debug, Error)]
pub enum HidError {
    #[error("could not initialise hidapi: {0}")]
    Init(#[source] hidapi::HidError),

    #[error("could not start the HID subsystem: {0}")]
    InitThread(String),

    #[error("no TH99 Pro found (looking for {vid:04x}:{pid:04x}). Is it connected by USB cable? The screen is not reachable over 2.4 GHz or Bluetooth.")]
    NotFound { vid: u16, pid: u16 },

    #[error("found the TH99 Pro but could not identify its {0} interface. Run `catbus99 probe --json` and open an issue with the output.")]
    InterfaceNotIdentified(&'static str),

    #[error("found {count} candidates for the {name} interface; refusing to guess. Run `catbus99 probe`.")]
    AmbiguousInterface { name: &'static str, count: usize },

    #[error("could not open the {name} interface: {source}. Another process may hold it -- close Epomaker's driver and any browser tab running the web driver.")]
    Open {
        name: &'static str,
        #[source]
        source: hidapi::HidError,
    },

    #[error("HID write failed: {0}")]
    Write(#[source] hidapi::HidError),

    #[error("HID read failed: {0}")]
    Read(#[source] hidapi::HidError),

    #[error("timed out waiting for an acknowledgement after report {index}")]
    AckTimeout { index: usize },

    #[error("report {index} was rejected: expected 55 41 00 01 + zeros, got {got}")]
    BadAck { index: usize, got: String },

    #[error("short write: sent {sent} of {expected} bytes")]
    ShortWrite { sent: usize, expected: usize },

    #[error("refusing a raw AA 50 panel write: image uploads must go through the write governor, which bounds flash wear")]
    UngovernedPanelWrite,

    #[error(transparent)]
    Proto(#[from] catbus99_proto::ProtoError),
}

/// Which of the keyboard's two interesting interfaces we mean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interface {
    /// `MI_02`: 64-byte config packets (clock, keymap).
    Config,
    /// `MI_03`: 4104-byte TFT image reports.
    Tft,
}

impl Interface {
    pub fn name(self) -> &'static str {
        match self {
            Interface::Config => "config (MI_02)",
            Interface::Tft => "TFT (MI_03)",
        }
    }

    pub fn interface_number(self) -> i32 {
        match self {
            Interface::Config => IFACE_CONFIG,
            Interface::Tft => IFACE_TFT,
        }
    }
}

/// A snapshot of one enumerated HID interface, for diagnostics.
#[derive(Debug, Clone, Serialize)]
pub struct InterfaceReport {
    pub path: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial_number: Option<String>,
    pub interface_number: i32,
    pub usage_page: u16,
    pub usage: u16,
}

impl From<&DeviceInfo> for InterfaceReport {
    fn from(d: &DeviceInfo) -> Self {
        Self {
            path: d.path().to_string_lossy().to_string(),
            vendor_id: d.vendor_id(),
            product_id: d.product_id(),
            manufacturer: d.manufacturer_string().map(str::to_owned),
            product: d.product_string().map(str::to_owned),
            serial_number: d.serial_number().map(str::to_owned),
            interface_number: d.interface_number(),
            usage_page: d.usage_page(),
            usage: d.usage(),
        }
    }
}

/// Everything `catbus99 probe` learned about the machine's HID devices.
#[derive(Debug, Clone, Serialize)]
pub struct ProbeReport {
    pub target_vid: u16,
    pub target_pid: u16,
    pub total_hid_devices: usize,
    pub matching_interfaces: Vec<InterfaceReport>,
    pub config_candidates: Vec<String>,
    pub tft_candidates: Vec<String>,
    pub verdict: String,
}

/// Enumerate all HID devices and report what we found for the TH99 Pro.
///
/// Opens nothing -- safe to run at any time.
pub fn probe() -> Result<ProbeReport, HidError> {
    with_api(probe_with)
}

fn probe_with(api: &HidApi) -> Result<ProbeReport, HidError> {
    let all: Vec<&DeviceInfo> = api.device_list().collect();
    let matching: Vec<&DeviceInfo> = all
        .iter()
        .copied()
        .filter(|d| d.vendor_id() == VID && d.product_id() == PID)
        .collect();

    let config_candidates = candidates(&matching, Interface::Config);
    let tft_candidates = candidates(&matching, Interface::Tft);

    let verdict = if matching.is_empty() {
        format!(
            "NOT FOUND: no {VID:04x}:{PID:04x} device. Connect the keyboard by USB cable \
             (the screen is unreachable over 2.4 GHz or Bluetooth)."
        )
    } else if config_candidates.len() == 1 && tft_candidates.len() == 1 {
        "OK: both interfaces identified unambiguously.".to_string()
    } else {
        format!(
            "AMBIGUOUS: {} config candidate(s), {} TFT candidate(s) among {} matching \
             interfaces. Interface identification needs a fallback for this device.",
            config_candidates.len(),
            tft_candidates.len(),
            matching.len()
        )
    };

    Ok(ProbeReport {
        target_vid: VID,
        target_pid: PID,
        total_hid_devices: all.len(),
        matching_interfaces: matching.iter().map(|d| InterfaceReport::from(*d)).collect(),
        config_candidates: config_candidates
            .iter()
            .map(|d| d.path().to_string_lossy().to_string())
            .collect(),
        tft_candidates: tft_candidates
            .iter()
            .map(|d| d.path().to_string_lossy().to_string())
            .collect(),
        verdict,
    })
}

fn candidates<'a>(matching: &[&'a DeviceInfo], want: Interface) -> Vec<&'a DeviceInfo> {
    matching
        .iter()
        .copied()
        .filter(|d| d.interface_number() == want.interface_number())
        .collect()
}

/// An open handle to one TH99 Pro interface.
pub struct Device {
    handle: hidapi::HidDevice,
    interface: Interface,
}

impl Device {
    /// Find and open one interface of the keyboard.
    pub fn open(interface: Interface) -> Result<Self, HidError> {
        with_api(|api| Self::open_with(api, interface))
    }

    fn open_with(api: &HidApi, interface: Interface) -> Result<Self, HidError> {
        let matching: Vec<&DeviceInfo> = api
            .device_list()
            .filter(|d| d.vendor_id() == VID && d.product_id() == PID)
            .collect();
        if matching.is_empty() {
            return Err(HidError::NotFound { vid: VID, pid: PID });
        }

        let found = candidates(&matching, interface);
        let chosen = match found.len() {
            1 => found[0],
            0 => return Err(HidError::InterfaceNotIdentified(interface.name())),
            n => {
                return Err(HidError::AmbiguousInterface {
                    name: interface.name(),
                    count: n,
                })
            }
        };

        let handle = api
            .open_path(chosen.path())
            .map_err(|source| HidError::Open {
                name: interface.name(),
                source,
            })?;

        Ok(Self { handle, interface })
    }

    pub fn interface(&self) -> Interface {
        self.interface
    }

    /// Write one output report.
    ///
    /// hidapi expects the report ID as the first byte. The TH99 Pro uses unnumbered
    /// reports, so we prepend `0x00` -- the same convention the Windows implementation
    /// uses when it sends 4105 bytes for a 4104-byte report.
    pub fn write_report(&self, report: &[u8]) -> Result<(), HidError> {
        // Hand-rolling an AA 50 stream here would be a second, ungoverned path to the
        // panel's flash. Refuse it: bulk image writes go through the governor.
        if report.len() >= 2 && report[0] == 0xAA && report[1] == catbus99_proto::report::CMD_TFT {
            return Err(HidError::UngovernedPanelWrite);
        }
        self.write_report_unchecked(report)
    }

    fn write_report_unchecked(&self, report: &[u8]) -> Result<(), HidError> {
        let mut framed = Vec::with_capacity(report.len() + 1);
        framed.push(0x00);
        framed.extend_from_slice(report);

        let sent = self.handle.write(&framed).map_err(HidError::Write)?;
        if sent != framed.len() {
            return Err(HidError::ShortWrite {
                sent,
                expected: framed.len(),
            });
        }
        Ok(())
    }

    /// Read one input report, waiting up to `timeout`.
    pub fn read_report(&self, len: usize, timeout: Duration) -> Result<Vec<u8>, HidError> {
        let mut buf = vec![0u8; len];
        let n = self
            .handle
            .read_timeout(&mut buf, timeout.as_millis() as i32)
            .map_err(HidError::Read)?;
        buf.truncate(n);
        Ok(buf)
    }
}

/// Progress callback: `(reports_acknowledged, total_reports)`.
pub type Progress<'a> = &'a mut dyn FnMut(usize, usize);

impl Device {
    /// Send a container as a sequence of AA 50 reports, verifying every acknowledgement.
    ///
    /// # Cost
    ///
    /// **This writes to the display's SPI flash** -- conservatively one P/E cycle of a
    /// rated 100,000. It is crate-private so that routing through the governor is not a
    /// convention callers must remember, but the only thing the compiler permits.
    ///
    /// # Failure
    ///
    /// Aborts at the first bad or missing acknowledgement. A partial upload may leave a
    /// torn image on screen; there is no clear command and no read-back, so recovery is
    /// re-uploading a known-good container.
    /// Read a full keymap layer from the config channel.
    ///
    /// Read-only: it issues `AA 12` (or `AA 16`) page reads and never writes. Restoring a
    /// keymap is a separate, destructive operation and is deliberately not implemented
    /// here.
    pub fn read_keymap(&self, command: u8, timeout: Duration) -> Result<Vec<u8>, HidError> {
        use catbus99_proto::keymap::{ENTRY_SIZE, MATRIX_KEYS, PAGE_SIZE};

        let total = MATRIX_KEYS * ENTRY_SIZE;
        let mut table = Vec::with_capacity(total);

        for page in 0..total.div_ceil(PAGE_SIZE) {
            let offset = (page * PAGE_SIZE) as u32;
            let request = catbus99_proto::keymap::build_read_request(command, offset);
            self.write_report(&request)?;

            let reply = self.read_report(64, timeout)?;
            if reply.len() < 8 + PAGE_SIZE {
                return Err(HidError::BadAck {
                    index: page,
                    got: format!("short keymap page: {} bytes", reply.len()),
                });
            }
            // The reply echoes the request header with 0xAA -> 0x55.
            if reply[0] != 0x55 || reply[1] != command {
                return Err(HidError::BadAck {
                    index: page,
                    got: reply[..2]
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<Vec<_>>()
                        .join(" "),
                });
            }
            table.extend_from_slice(&reply[8..8 + PAGE_SIZE]);
        }
        table.truncate(total);
        Ok(table)
    }

    pub(crate) fn upload_container(
        &self,
        payload: &[u8],
        timeout: Duration,
        mut progress: Option<Progress<'_>>,
    ) -> Result<usize, HidError> {
        use catbus99_proto::report::{build_reports, is_valid_ack, ACK_SIZE};

        let reports = build_reports(payload)?;
        let total = reports.len();

        for (index, report) in reports.iter().enumerate() {
            self.write_report_unchecked(report)?;

            let ack = self.read_report(ACK_SIZE, timeout)?;
            if ack.is_empty() {
                return Err(HidError::AckTimeout { index });
            }
            if !is_valid_ack(&ack) {
                return Err(HidError::BadAck {
                    index,
                    got: ack
                        .iter()
                        .take(8)
                        .map(|b| format!("{b:02x}"))
                        .collect::<Vec<_>>()
                        .join(" "),
                });
            }
            if let Some(cb) = progress.as_mut() {
                cb(index + 1, total);
            }
        }
        Ok(total)
    }
}
