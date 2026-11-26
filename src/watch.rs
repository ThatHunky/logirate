//! The background loop: follow what's running and keep the mouse at the configured rate.

use crate::config::{Config, Target};
use crate::foreground::{foreground, running_exes};
use crate::hidpp::Device;
use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

/// Set when the service is shutting down: the loop stops touching the mouse.
pub static STOP: AtomicBool = AtomicBool::new(false);
/// Held while the loop talks to the mouse, so a shutdown can wait for an in-flight change.
static DEVICE_LOCK: Mutex<()> = Mutex::new(());

/// Stops the loop and hands the mouse back to its onboard profile.
pub fn shutdown(log: &mut dyn FnMut(&str)) {
    STOP.store(true, Ordering::SeqCst);
    let _guard = DEVICE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    match Device::open().and_then(|mut d| d.use_onboard()) {
        Ok(()) => log("stopping: mouse handed back to its onboard profile"),
        Err(e) => log(&format!("stopping: couldn't reset the mouse: {e:#}")),
    }
}

const VERIFY_EVERY: Duration = Duration::from_secs(3);
const REOPEN_EVERY: Duration = Duration::from_secs(3);

pub fn run(log: &mut dyn FnMut(&str)) -> ! {
    let mut config = load(log);
    let mut config_mtime = mtime();
    let mut device: Option<Device> = None;
    let mut applied: Option<Target> = None;
    let mut last_verify = Instant::now();
    let mut last_open = Instant::now() - REOPEN_EVERY;
    let mut last_error = String::new();

    log(&format!("logirate started, config {}", Config::path().display()));
    loop {
        std::thread::sleep(Duration::from_millis(config.poll_ms));
        if STOP.load(Ordering::SeqCst) {
            std::thread::park();
            continue;
        }

        if mtime() != config_mtime {
            config_mtime = mtime();
            config = load(log);
            applied = None;
        }

        let fg = foreground();
        let running = if config.needs_process_list() { running_exes() } else { HashSet::new() };
        let (target, rule) = config.target_for(fg.as_ref(), &running);
        let why = match rule {
            Some(r) => format!("rule {}", r.name),
            None => format!("default, {}", fg.as_ref().map(|f| f.exe.as_str()).unwrap_or("no window")),
        };

        let verify_due = matches!(target, Target::Host { .. }) && last_verify.elapsed() >= VERIFY_EVERY;
        if applied == Some(target) && !verify_due {
            continue;
        }

        let _guard = DEVICE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        if STOP.load(Ordering::SeqCst) {
            continue;
        }
        if device.is_none() {
            if last_open.elapsed() < REOPEN_EVERY {
                continue;
            }
            last_open = Instant::now();
            match Device::open() {
                Ok(d) => {
                    log(&format!("mouse found on {} (device {})", d.receiver(), d.index));
                    device = Some(d);
                    applied = None;
                }
                Err(e) => {
                    note_error(log, &mut last_error, &format!("{e:#}"));
                    continue;
                }
            }
        }
        let dev = device.as_mut().unwrap();
        last_verify = Instant::now();

        let result = match (applied == Some(target), target) {
            // Periodic check while in host mode.
            (true, Target::Host { hz, dpi }) => match dev.is_at(hz, dpi) {
                Ok(true) => Ok(false),
                Ok(false) => dev.set_host(hz, dpi).map(|_| true),
                Err(e) => Err(e),
            },
            (true, Target::Onboard) => Ok(false),
            (false, Target::Host { hz, dpi }) => dev.set_host(hz, dpi).map(|_| true),
            (false, Target::Onboard) => dev.use_onboard().map(|_| true),
        };

        match result {
            Ok(changed) => {
                if changed {
                    log(&format!("-> {target}  [{why}]"));
                }
                applied = Some(target);
                last_error.clear();
            }
            Err(e) => {
                note_error(log, &mut last_error, &format!("{e:#}"));
                device = None;
                applied = None;
            }
        }
    }
}

fn note_error(log: &mut dyn FnMut(&str), last: &mut String, msg: &str) {
    if last != msg {
        log(&format!("error: {msg}"));
        *last = msg.to_string();
    }
}

fn load(log: &mut dyn FnMut(&str)) -> Config {
    match Config::load() {
        Ok(c) => {
            let rules: Vec<_> = c.rules.iter().map(|r| format!("{} -> {}", r.name, r.target)).collect();
            log(&format!("config: default {}; rules: {}", c.default, if rules.is_empty() { "none".into() } else { rules.join("; ") }));
            c
        }
        Err(e) => {
            log(&format!("config error, keeping the onboard profile everywhere: {e:#}"));
            Config { default: Target::Onboard, poll_ms: 500, rules: Vec::new() }
        }
    }
}

fn mtime() -> Option<SystemTime> {
    std::fs::metadata(Config::path()).and_then(|m| m.modified()).ok()
}
