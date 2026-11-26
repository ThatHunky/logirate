//! logirate-gui.exe: a small window to see the mouse, edit rules and manage the background service.
#![windows_subsystem = "windows"]

use eframe::egui;
use logirate::config::{Config, Rule, Target, VALID_RATES, When};
use logirate::foreground::running_exes;
use logirate::hidpp::Device;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

const NO_WINDOW: u32 = 0x0800_0000;
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("logirate")
            .with_icon(std::sync::Arc::new(
                eframe::icon_data::from_png_bytes(include_bytes!("../assets/logirate-256.png")).unwrap_or_default(),
            ))
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([900.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native(
        "logirate",
        options,
        Box::new(|cc| {
            setup_style(&cc.egui_ctx);
            Ok(Box::new(App::new()))
        }),
    )
}

/// Bigger text, roomier controls and higher contrast than egui's compact defaults.
fn setup_style(ctx: &egui::Context) {
    ctx.set_zoom_factor(1.2);
    ctx.all_styles_mut(|style| {
        use egui::{FontId, TextStyle};
        style.text_styles = [
            (TextStyle::Heading, FontId::proportional(20.0)),
            (TextStyle::Body, FontId::proportional(15.0)),
            (TextStyle::Button, FontId::proportional(15.0)),
            (TextStyle::Small, FontId::proportional(12.5)),
            (TextStyle::Monospace, FontId::monospace(13.0)),
        ]
        .into();
        style.spacing.item_spacing = egui::vec2(10.0, 8.0);
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        style.spacing.interact_size.y = 28.0;
        style.visuals.override_text_color = Some(if style.visuals.dark_mode {
            egui::Color32::from_gray(232)
        } else {
            egui::Color32::from_gray(20)
        });
    });
}

#[derive(Clone, Default)]
struct EditRule {
    name: String,
    processes: String,
    title: String,
    running: bool,
    rate: Option<u32>,
    dpi: String,
}

impl EditRule {
    fn from_rule(r: &Rule) -> Self {
        let (rate, dpi) = split(r.target);
        EditRule {
            name: r.name.clone(),
            processes: r.processes.join(", "),
            title: r.title.clone().unwrap_or_default(),
            running: r.when == When::Running,
            rate,
            dpi,
        }
    }

    fn to_rule(&self, n: usize) -> Result<Rule, String> {
        let processes: Vec<String> =
            self.processes.split(',').map(|p| p.trim().to_lowercase()).filter(|p| !p.is_empty()).collect();
        let title = Some(self.title.trim().to_lowercase()).filter(|t| !t.is_empty());
        if processes.is_empty() && (title.is_none() || self.running) {
            return Err(format!("rule {n}: add an app (or a window title for focused rules)"));
        }
        let target = join(self.rate, &self.dpi).map_err(|e| format!("rule {n}: {e}"))?;
        let name = if self.name.trim().is_empty() { processes.join(", ") } else { self.name.trim().to_string() };
        Ok(Rule { name, processes, title, when: if self.running { When::Running } else { When::Focused }, target })
    }
}

fn split(t: Target) -> (Option<u32>, String) {
    match t {
        Target::Onboard => (None, String::new()),
        Target::Host { hz, dpi } => (Some(hz), dpi.map(|d| d.to_string()).unwrap_or_default()),
    }
}

fn join(rate: Option<u32>, dpi: &str) -> Result<Target, String> {
    let dpi = match dpi.trim() {
        "" => None,
        d => Some(d.parse::<u16>().map_err(|_| format!("DPI \"{d}\" isn't a number"))?),
    };
    Ok(match rate {
        None => Target::Onboard,
        Some(hz) => Target::Host { hz, dpi },
    })
}

struct App {
    rules: Vec<EditRule>,
    default_rate: Option<u32>,
    default_dpi: String,
    poll_ms: u64,
    saved_snapshot: String,
    message: String,
    mouse: String,
    running_apps: Vec<String>,
    pick: String,
    service_running: bool,
    autostart: bool,
    log_tail: String,
    last_poll: Instant,
}

impl App {
    fn new() -> Self {
        let mut app = App {
            rules: Vec::new(),
            default_rate: None,
            default_dpi: String::new(),
            poll_ms: 500,
            saved_snapshot: String::new(),
            message: String::new(),
            mouse: String::new(),
            running_apps: Vec::new(),
            pick: String::new(),
            service_running: false,
            autostart: autostart_enabled(),
            log_tail: String::new(),
            last_poll: Instant::now() - Duration::from_secs(60),
        };
        app.reload();
        app.refresh_mouse();
        app
    }

    fn reload(&mut self) {
        match Config::load() {
            Ok(c) => {
                self.rules = c.rules.iter().map(EditRule::from_rule).collect();
                (self.default_rate, self.default_dpi) = split(c.default);
                self.poll_ms = c.poll_ms;
                self.saved_snapshot = self.snapshot();
                self.message = format!("Loaded {}", Config::path().display());
            }
            Err(e) => self.message = format!("Couldn't read the config: {e:#}"),
        }
    }

    fn snapshot(&self) -> String {
        let mut s = format!("{:?}|{}|{}", self.default_rate, self.default_dpi, self.poll_ms);
        for r in &self.rules {
            s += &format!("|{}~{}~{}~{}~{:?}~{}", r.name, r.processes, r.title, r.running, r.rate, r.dpi);
        }
        s
    }

    fn build_config(&self) -> Result<Config, String> {
        let rules = self.rules.iter().enumerate().map(|(i, r)| r.to_rule(i + 1)).collect::<Result<Vec<_>, _>>()?;
        Ok(Config { default: join(self.default_rate, &self.default_dpi)?, poll_ms: self.poll_ms, rules })
    }

    fn save(&mut self) {
        match self.build_config().and_then(|c| c.save().map_err(|e| format!("{e:#}"))) {
            Ok(()) => {
                self.saved_snapshot = self.snapshot();
                self.message = if self.service_running {
                    "Saved. The background service picks it up within a second.".into()
                } else {
                    "Saved. Start the background service to use it.".into()
                };
            }
            Err(e) => self.message = format!("Not saved: {e}"),
        }
    }

    fn refresh_mouse(&mut self) {
        self.mouse = match Device::open() {
            Ok(mut d) => {
                let name = d.name().unwrap_or_else(|_| "mouse".into());
                let rate = d.rate().map(|r| format!("{r} Hz")).unwrap_or("? Hz".into());
                let mode = d.mode().ok().flatten().map(|m| m.to_string()).unwrap_or("-".into());
                let dpi = d.dpi().map(|v| format!("{v} DPI")).unwrap_or_default();
                let rates = d.supported_rates().map(|r| format!("{r:?}")).unwrap_or_default();
                format!("{name} on {}: {rate}, {dpi}, {mode} mode. Supports {rates} Hz", d.receiver())
            }
            Err(e) => format!("Mouse not reachable: {e:#}"),
        };
    }

    fn poll(&mut self) {
        if self.last_poll.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.last_poll = Instant::now();
        let procs = running_exes();
        self.service_running = procs.contains("logirated.exe");
        let mut apps: Vec<String> = procs.into_iter().filter(|p| p.ends_with(".exe")).collect();
        apps.sort();
        self.running_apps = apps;
        self.log_tail = read_log_tail(14);
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll();
        ui.ctx().request_repaint_after(Duration::from_secs(2));

        egui::Panel::top("status").show(ui, |ui| {
            ui.add_space(10.0);
            ui.heading("logirate");
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.strong("Mouse");
                ui.label(&self.mouse);
                if ui.button("Refresh").clicked() {
                    self.refresh_mouse();
                }
            });
            ui.horizontal(|ui| {
                ui.strong("Background service");
                if self.service_running {
                    ui.colored_label(egui::Color32::from_rgb(60, 160, 90), "running");
                    if ui.button("Stop").clicked() {
                        stop_service();
                        // The service can't clean up after a forced stop; hand the mouse back ourselves.
                        self.message = match Device::open().and_then(|mut d| d.use_onboard()) {
                            Ok(()) => "Service stopped; the mouse is back on its onboard profile.".into(),
                            Err(e) => format!("Service stopped, but the mouse couldn't be reset: {e:#}"),
                        };
                        self.refresh_mouse();
                        self.last_poll = Instant::now() - Duration::from_secs(60);
                    }
                } else {
                    ui.colored_label(egui::Color32::from_rgb(200, 120, 40), "stopped");
                    if ui.button("Start").clicked() {
                        self.message = start_service().err().unwrap_or_else(|| "Service started.".into());
                        self.last_poll = Instant::now() - Duration::from_secs(60);
                    }
                }
                ui.separator();
                let mut on = self.autostart;
                if ui.checkbox(&mut on, "Start at login").changed() {
                    match set_autostart(on) {
                        Ok(()) => self.autostart = on,
                        Err(e) => self.message = e,
                    }
                }
            });
            ui.add_space(6.0);
        });

        egui::Panel::bottom("log").resizable(true).default_size(150.0).show(ui, |ui| {
            ui.add_space(4.0);
            ui.strong("Service log");
            egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
                ui.add(egui::Label::new(egui::RichText::new(&self.log_tail).monospace()).wrap());
            });
        });

        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("When no rule matches:");
                rate_combo(ui, "default-rate", &mut self.default_rate);
                if self.default_rate.is_some() {
                    ui.label("DPI");
                    ui.add(egui::TextEdit::singleline(&mut self.default_dpi).desired_width(80.0).hint_text("keep"));
                }
                ui.separator();
                ui.label("Check every");
                ui.add(egui::DragValue::new(&mut self.poll_ms).range(100..=5000).suffix(" ms"));
            });
            ui.add_space(12.0);
            ui.heading("Rules");
            ui.label(
                egui::RichText::new(
                    "Checked top to bottom; the first match wins. Apps: comma-separated, * and ? work, \
                     a pattern with \\ matches the full path. \"onboard\" hands control back to the mouse's own profile.",
                )
                .small(),
            );
            ui.add_space(6.0);

            let mut remove = None;
            let mut swap = None;
            let count = self.rules.len();
            // Text columns share whatever width is left after the fixed-size controls.
            let flexible = (ui.available_width() - 520.0).max(420.0);
            let (w_name, w_apps, w_title) = (flexible * 0.2, flexible * 0.5, flexible * 0.3);
            egui::ScrollArea::vertical().max_height((ui.available_height() - 70.0).max(80.0)).show(ui, |ui| {
                egui::Grid::new("rules").striped(true).num_columns(7).spacing([12.0, 10.0]).show(ui, |ui| {
                    for h in ["Name", "Apps", "Window title contains", "Active", "Rate", "DPI", ""] {
                        ui.strong(h);
                    }
                    ui.end_row();
                    for (i, r) in self.rules.iter_mut().enumerate() {
                        // add_sized: a Grid otherwise caps each cell at last frame's (narrow) column width.
                        let h = ui.spacing().interact_size.y;
                        ui.add_sized([w_name, h], egui::TextEdit::singleline(&mut r.name).hint_text("optional"));
                        ui.add_sized([w_apps, h], egui::TextEdit::singleline(&mut r.processes).hint_text("cs2.exe, valorant*.exe"));
                        ui.add_sized([w_title, h], egui::TextEdit::singleline(&mut r.title).hint_text("optional"));
                        egui::ComboBox::from_id_salt(("when", i))
                            .width(130.0)
                            .selected_text(if r.running { "while running" } else { "when focused" })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut r.running, false, "when focused");
                                ui.selectable_value(&mut r.running, true, "while running");
                            });
                        rate_combo(ui, ("rate", i), &mut r.rate);
                        ui.add_enabled_ui(r.rate.is_some(), |ui| {
                            ui.add_sized([80.0, h], egui::TextEdit::singleline(&mut r.dpi).hint_text("keep"));
                        });
                        ui.horizontal(|ui| {
                            if ui.add_enabled(i > 0, egui::Button::new("Up")).clicked() {
                                swap = Some((i, i - 1));
                            }
                            if ui.add_enabled(i + 1 < count, egui::Button::new("Down")).clicked() {
                                swap = Some((i, i + 1));
                            }
                            if ui.button("Remove").clicked() {
                                remove = Some(i);
                            }
                        });
                        ui.end_row();
                    }
                });
                if self.rules.is_empty() {
                    ui.label("No rules yet. Add one below, e.g. pick cs2.exe while the game is running.");
                }
            });
            if let Some((a, b)) = swap {
                self.rules.swap(a, b);
            }
            if let Some(i) = remove {
                self.rules.remove(i);
            }

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Add rule").clicked() {
                    self.rules.push(EditRule { rate: Some(8000), ..Default::default() });
                }
                ui.separator();
                ui.label("Add a running app:");
                egui::ComboBox::from_id_salt("pick")
                    .width(280.0)
                    .selected_text(if self.pick.is_empty() { "choose…" } else { self.pick.as_str() })
                    .show_ui(ui, |ui| {
                        for app in &self.running_apps {
                            ui.selectable_value(&mut self.pick, app.clone(), app);
                        }
                    });
                if ui.add_enabled(!self.pick.is_empty(), egui::Button::new("Add")).clicked() {
                    self.rules.push(EditRule { processes: self.pick.clone(), rate: Some(8000), ..Default::default() });
                    self.pick.clear();
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let dirty = self.snapshot() != self.saved_snapshot;
                if ui.add_enabled(dirty, egui::Button::new("Save")).clicked() {
                    self.save();
                }
                if ui.add_enabled(dirty, egui::Button::new("Revert")).clicked() {
                    self.reload();
                }
                if dirty {
                    ui.label("Unsaved changes");
                }
                ui.separator();
                ui.label(&self.message);
            });
        });
    }
}

fn rate_combo(ui: &mut egui::Ui, id: impl egui::AsIdSalt, rate: &mut Option<u32>) {
    let label = |r: Option<u32>| r.map(|hz| format!("{hz} Hz")).unwrap_or_else(|| "onboard".into());
    egui::ComboBox::from_id_salt(id).width(115.0).selected_text(label(*rate)).show_ui(ui, |ui| {
        ui.selectable_value(rate, None, "onboard");
        for hz in VALID_RATES {
            ui.selectable_value(rate, Some(hz), format!("{hz} Hz"));
        }
    });
}

fn exe_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(PathBuf::from)).unwrap_or_default()
}

fn daemon_path() -> PathBuf {
    exe_dir().join("logirated.exe")
}

fn start_service() -> Result<(), String> {
    Command::new(daemon_path())
        .creation_flags(NO_WINDOW)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Couldn't start {}: {e}", daemon_path().display()))
}

/// Asks the service to close (it resets the mouse itself); force-kills it if it doesn't go.
fn stop_service() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW, WM_CLOSE};
    let class: Vec<u16> = "logirated-window".encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let hwnd = FindWindowW(class.as_ptr(), std::ptr::null());
        if !hwnd.is_null() {
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
        }
    }
    for _ in 0..30 {
        if !running_exes().contains("logirated.exe") {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = Command::new("taskkill").args(["/IM", "logirated.exe", "/F"]).creation_flags(NO_WINDOW).status();
}

fn autostart_enabled() -> bool {
    Command::new("reg")
        .args(["query", RUN_KEY, "/v", "logirate"])
        .creation_flags(NO_WINDOW)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn set_autostart(on: bool) -> Result<(), String> {
    let status = if on {
        let value = format!("\"{}\"", daemon_path().display());
        Command::new("reg").args(["add", RUN_KEY, "/v", "logirate", "/t", "REG_SZ", "/d", &value, "/f"]).creation_flags(NO_WINDOW).status()
    } else {
        Command::new("reg").args(["delete", RUN_KEY, "/v", "logirate", "/f"]).creation_flags(NO_WINDOW).status()
    };
    match status {
        Ok(s) if s.success() => Ok(()),
        _ => Err("Couldn't change the login setting".into()),
    }
}

fn read_log_tail(lines: usize) -> String {
    match std::fs::read_to_string(logirate::paths::log_file()) {
        Ok(text) => {
            let all: Vec<&str> = text.lines().collect();
            all[all.len().saturating_sub(lines)..].join("\n")
        }
        Err(_) => "No log yet. Start the background service.".into(),
    }
}
