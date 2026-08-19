//! Regression tests for the macOS hidapi threading constraint.
//!
//! libhidapi ties its IOKit state to the thread that calls `hid_init()`. If that thread
//! exits, later use aborts the process. These tests would have caught that: before the
//! dedicated init thread, both of them crashed with SIGTRAP rather than failing.

/// The original failure: concurrent probes, where initialisation lands on a short-lived
/// worker thread.
#[test]
fn concurrent_probes_from_short_lived_threads() {
    let handles: Vec<_> = (0..8)
        .map(|_| std::thread::spawn(|| catbus99_device::probe().is_ok()))
        .collect();
    for h in handles {
        h.join().expect("a probe thread panicked");
    }
}

/// Initialise from a thread that then exits, and use the handle afterwards from a
/// different thread — the exact shape that aborts without the dedicated init thread.
#[test]
fn a_probe_survives_the_death_of_its_initialising_thread() {
    std::thread::spawn(|| {
        let _ = catbus99_device::probe();
    })
    .join()
    .expect("initialising thread panicked");

    // The thread that first touched hidapi is now gone.
    std::thread::spawn(|| {
        catbus99_device::probe().expect("probe after initialising thread exited");
    })
    .join()
    .expect("second thread panicked");
}

/// Opening and probing interleaved, as the daemon and an MCP client can do.
#[test]
fn probing_and_opening_interleaved() {
    let probes: Vec<_> = (0..4)
        .map(|_| std::thread::spawn(|| catbus99_device::probe().is_ok()))
        .collect();
    let opens: Vec<_> = (0..4)
        .map(|_| {
            std::thread::spawn(|| {
                // Failing to open is fine without hardware; aborting is not.
                catbus99_device::Device::open(catbus99_device::Interface::Tft).is_ok()
            })
        })
        .collect();
    for h in probes {
        h.join().expect("probe thread panicked");
    }
    for h in opens {
        h.join().expect("open thread panicked");
    }
}

/// A device plugged in after start-up must still be found: the shared handle is
/// long-lived, so enumeration has to refresh rather than reuse a start-up snapshot.
#[test]
fn repeated_probes_re_enumerate() {
    let a = catbus99_device::probe().expect("first probe");
    let b = catbus99_device::probe().expect("second probe");
    assert_eq!(a.total_hid_devices, b.total_hid_devices);
}

#[test]
fn explicit_init_is_idempotent() {
    catbus99_device::init().expect("init");
    catbus99_device::init().expect("init again");
    catbus99_device::probe().expect("probe after init");
}
