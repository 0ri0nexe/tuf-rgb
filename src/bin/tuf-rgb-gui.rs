//! tuf-rgb-gui: graphical editor for effects and profiles. Drives tuf-rgbd over its socket.

use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};
use tuf_rgb::Rgb;
use tuf_rgb::effects::{Effect, EffectSpec};
use tuf_rgb::hw::{HwMode, PowerState, Speed};
use tuf_rgb::ipc::{self, Request, Status};

fn main() -> eframe::Result<()> {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([860.0, 620.0])
            .with_title("TUF RGB")
            .with_app_id("tuf-rgb"),
        // A blocking eglSwapBuffers waits for a Wayland frame callback that Hyprland may never
        // send, which froze the event loop ("Application Not Responding"). We pace frames ourselves.
        vsync: false,
        ..Default::default()
    };
    eframe::run_native("tuf-rgb", opts, Box::new(|_cc| Ok(Box::new(App::new()))))
}

struct App {
    status: Option<Status>,
    error: Option<String>,
    last_poll: Instant,
    spec: EffectSpec,
    preview: Box<dyn Effect>,
    preview_t0: Instant,
    preview_last: Instant,
    live: bool,
    dirty_since: Option<Instant>,
    new_profile: String,
    power: PowerState,
    power_save: bool,
    notify_color: Rgb,
}

fn c32(c: Rgb) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

fn color_edit(ui: &mut egui::Ui, c: &mut Rgb) -> bool {
    let mut a = [c.r, c.g, c.b];
    let changed = egui::color_picker::color_edit_button_srgb(ui, &mut a).changed();
    if changed {
        *c = Rgb::new(a[0], a[1], a[2]);
    }
    changed
}

fn labeled<R>(ui: &mut egui::Ui, label: &str, f: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.add_sized([110.0, 18.0], egui::Label::new(label));
        f(ui)
    })
    .inner
}

fn slider(ui: &mut egui::Ui, label: &str, v: &mut f64, range: std::ops::RangeInclusive<f64>, suffix: &str) -> bool {
    labeled(ui, label, |ui| ui.add(egui::Slider::new(v, range).suffix(suffix)).changed())
}

fn color_row(ui: &mut egui::Ui, label: &str, c: &mut Rgb) -> bool {
    labeled(ui, label, |ui| color_edit(ui, c))
}

fn palette(ui: &mut egui::Ui, colors: &mut Vec<Rgb>) -> bool {
    let mut changed = false;
    labeled(ui, "Colors", |ui| {
        let mut remove = None;
        let removable = colors.len() > 1;
        for (i, c) in colors.iter_mut().enumerate() {
            changed |= color_edit(ui, c);
            if removable && ui.small_button("×").on_hover_text("remove").clicked() {
                remove = Some(i);
            }
        }
        if let Some(i) = remove {
            colors.remove(i);
            changed = true;
        }
        if ui.button("+").clicked() {
            let next = colors.last().copied().unwrap_or(Rgb::new(255, 0, 0));
            colors.push(Rgb::from_hsv(rgb_hue(next) + 60.0, 1.0, 1.0));
            changed = true;
        }
    });
    changed
}

fn rgb_hue(c: Rgb) -> f64 {
    let (r, g, b) = c.to_f64();
    let max = r.max(g).max(b);
    let d = max - r.min(g).min(b);
    if d == 0.0 {
        return 0.0;
    }
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    h * 60.0
}

/// Parameter widgets for the selected effect. Returns true when something changed.
fn edit_spec(ui: &mut egui::Ui, spec: &mut EffectSpec) -> bool {
    let mut ch = false;
    match spec {
        EffectSpec::Hardware { mode, color, speed, save } => {
            labeled(ui, "Firmware mode", |ui| {
                egui::ComboBox::from_id_salt("hwmode").selected_text(mode.label()).show_ui(ui, |ui| {
                    for m in HwMode::NAMED {
                        ch |= ui.selectable_value(mode, m, m.label()).changed();
                    }
                    for b in [4u8, 5, 6, 7, 8, 10, 11] {
                        ch |= ui
                            .selectable_value(mode, HwMode::Raw(b), format!("Raw {b} (experimental)"))
                            .changed();
                    }
                });
            });
            ch |= color_row(ui, "Color", color);
            labeled(ui, "Speed", |ui| {
                for (s, n) in [(Speed::Slow, "Slow"), (Speed::Medium, "Medium"), (Speed::Fast, "Fast")] {
                    ch |= ui.selectable_value(speed, s, n).changed();
                }
            });
            ch |= labeled(ui, "Persist", |ui| ui.checkbox(save, "save to EC (survives reboot)").changed());
            ui.label(RichText::new("Rendered by the keyboard controller, zero CPU usage.").weak());
        }
        EffectSpec::Solid { color } => ch |= color_row(ui, "Color", color),
        EffectSpec::Breathe { colors, period, min } => {
            ch |= palette(ui, colors);
            ch |= slider(ui, "Period", period, 0.3..=20.0, " s");
            ch |= slider(ui, "Min level", min, 0.0..=1.0, "");
        }
        EffectSpec::Rainbow { period, saturation, value } => {
            ch |= slider(ui, "Period", period, 0.5..=60.0, " s");
            ch |= slider(ui, "Saturation", saturation, 0.0..=1.0, "");
            ch |= slider(ui, "Value", value, 0.0..=1.0, "");
        }
        EffectSpec::Gradient { colors, period } => {
            ch |= palette(ui, colors);
            ch |= slider(ui, "Loop period", period, 0.5..=60.0, " s");
        }
        EffectSpec::Cycle { colors, hold, fade } => {
            ch |= palette(ui, colors);
            ch |= slider(ui, "Hold", hold, 0.0..=10.0, " s");
            ch |= slider(ui, "Fade", fade, 0.0..=10.0, " s");
        }
        EffectSpec::Strobe { colors, frequency, duty } => {
            ch |= palette(ui, colors);
            ch |= slider(ui, "Frequency", frequency, 0.2..=15.0, " Hz");
            ch |= slider(ui, "Duty", duty, 0.05..=0.95, "");
        }
        EffectSpec::Heartbeat { color, bpm, min } => {
            ch |= color_row(ui, "Color", color);
            ch |= slider(ui, "BPM", bpm, 20.0..=200.0, "");
            ch |= slider(ui, "Min level", min, 0.0..=1.0, "");
        }
        EffectSpec::Candle { color, flicker } => {
            ch |= color_row(ui, "Flame", color);
            ch |= slider(ui, "Flicker", flicker, 0.0..=1.0, "");
        }
        EffectSpec::Police { a, b, period } => {
            ch |= color_row(ui, "Color A", a);
            ch |= color_row(ui, "Color B", b);
            ch |= slider(ui, "Period", period, 0.2..=5.0, " s");
        }
        EffectSpec::Storm { base, flash, rate } => {
            ch |= color_row(ui, "Sky", base);
            ch |= color_row(ui, "Lightning", flash);
            ch |= slider(ui, "Flashes", rate, 1.0..=120.0, " /min");
        }
        EffectSpec::CpuLoad { low, high } => {
            ch |= color_row(ui, "Idle", low);
            ch |= color_row(ui, "Busy", high);
        }
        EffectSpec::Temperature { cold, hot, min_c, max_c, sensor } => {
            ch |= color_row(ui, "Cold", cold);
            ch |= color_row(ui, "Hot", hot);
            ch |= slider(ui, "Cold at", min_c, 20.0..=100.0, " °C");
            ch |= slider(ui, "Hot at", max_c, 20.0..=110.0, " °C");
            let mut s = sensor.clone().unwrap_or_default();
            if labeled(ui, "Sensor", |ui| ui.text_edit_singleline(&mut s).changed()) {
                *sensor = (!s.trim().is_empty()).then(|| s.trim().to_string());
                ch = true;
            }
            ui.label(RichText::new("Sensor: hwmon name or thermal zone type, empty = CPU package.").weak());
        }
        EffectSpec::Battery { empty, full } => {
            ch |= color_row(ui, "Empty", empty);
            ch |= color_row(ui, "Full", full);
            ui.label(RichText::new("Breathes while charging.").weak());
        }
        EffectSpec::Off => {}
    }
    ch
}

impl App {
    fn new() -> Self {
        let spec = EffectSpec::default_for("rainbow").unwrap();
        let mut app = Self {
            preview: spec.build(),
            spec,
            status: None,
            error: None,
            last_poll: Instant::now() - Duration::from_secs(10),
            preview_t0: Instant::now(),
            preview_last: Instant::now(),
            live: true,
            dirty_since: None,
            new_profile: String::new(),
            power: PowerState::default(),
            power_save: false,
            notify_color: Rgb::new(0, 255, 0),
        };
        app.poll();
        if let Some(e) = app.status.as_ref().and_then(|s| s.effect.clone()) {
            app.set_spec(e);
        }
        if let Some(p) = app.status.as_ref().and_then(|s| s.power) {
            app.power = p;
        }
        app
    }

    fn set_spec(&mut self, spec: EffectSpec) {
        self.preview = spec.build();
        self.preview_t0 = Instant::now();
        self.spec = spec;
    }

    fn send(&mut self, req: Request) {
        match ipc::send(&req) {
            Ok(r) => {
                if r.status.is_some() {
                    self.status = r.status;
                }
                self.error = None;
            }
            Err(e) => self.error = Some(format!("{e:#}")),
        }
    }

    fn poll(&mut self) {
        self.last_poll = Instant::now();
        match ipc::send(&Request::Status) {
            Ok(r) => {
                self.status = r.status;
                if self.error.as_deref().is_some_and(|e| e.contains("not reachable")) {
                    self.error = None;
                }
            }
            Err(e) => {
                self.status = None;
                self.error = Some(format!("{e:#}"));
            }
        }
    }

    fn start_daemon(&mut self) {
        let exe = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("tuf-rgbd")));
        let cmd = exe.filter(|p| p.exists()).unwrap_or_else(|| "tuf-rgbd".into());
        // Right after install.sh the session does not have the tufrgb group yet; sg grants it.
        let writable = std::fs::OpenOptions::new()
            .write(true)
            .open(format!("{}/kbd_rgb_mode", tuf_rgb::hw::DEFAULT_LED))
            .is_ok();
        let mut command = if writable {
            std::process::Command::new(&cmd)
        } else {
            let mut c = std::process::Command::new("sg");
            c.arg("tufrgb").arg("-c").arg(&cmd);
            c
        };
        match command.spawn() {
            Ok(_) => {
                std::thread::sleep(Duration::from_millis(300));
                self.poll();
            }
            Err(e) => self.error = Some(format!("cannot start {}: {e}", cmd.display())),
        }
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("TUF RGB");
            ui.separator();
            match &self.status {
                Some(st) => {
                    ui.label(RichText::new("● daemon").color(Color32::from_rgb(60, 200, 90)));
                    ui.label(RichText::new(&st.backend).weak());
                    if let Some(c) = st.color {
                        let (r, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
                        ui.painter().rect_filled(r, 4.0, c32(c));
                        ui.label(c.hex());
                    }
                }
                None => {
                    ui.label(RichText::new("● daemon offline").color(Color32::from_rgb(220, 80, 60)));
                    if ui.button("Start tuf-rgbd").clicked() {
                        self.start_daemon();
                    }
                }
            }
        });
        let err = self.error.clone().or_else(|| self.status.as_ref().and_then(|s| s.last_error.clone()));
        if let Some(e) = err {
            ui.colored_label(Color32::from_rgb(230, 120, 60), e);
        }
    }

    fn profiles_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Profiles");
        let Some(st) = self.status.clone() else {
            ui.label("Start the daemon to manage profiles.");
            return;
        };
        egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
            for name in &st.profiles {
                ui.horizontal(|ui| {
                    let active = st.active_profile.as_deref() == Some(name.as_str());
                    if ui.selectable_label(active, name).clicked() {
                        self.send(Request::Profile { name: name.clone() });
                        if let Some(e) = self.status.as_ref().and_then(|s| s.effect.clone()) {
                            self.set_spec(e);
                        }
                    }
                    if ui.small_button("🗑").on_hover_text("delete").clicked() {
                        self.send(Request::DeleteProfile { name: name.clone() });
                    }
                });
            }
        });
        ui.separator();
        ui.label("Save current effect as:");
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.new_profile).desired_width(110.0));
            let name = self.new_profile.trim().to_string();
            if ui.add_enabled(!name.is_empty(), egui::Button::new("Save")).clicked() {
                self.send(Request::SaveProfile { name: name.clone(), effect: self.spec.clone() });
                self.send(Request::Profile { name });
                self.new_profile.clear();
            }
        });
    }

    fn effect_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Effect");
        ui.horizontal_wrapped(|ui| {
            for proto in EffectSpec::catalog() {
                let k = proto.kind();
                if ui.selectable_label(self.spec.kind() == k, k).clicked() && self.spec.kind() != k {
                    self.set_spec(proto);
                    self.dirty_since = Some(Instant::now());
                }
            }
        });
        ui.separator();
        let mut spec = self.spec.clone();
        if edit_spec(ui, &mut spec) {
            self.set_spec(spec);
            self.dirty_since = Some(Instant::now());
        }
        ui.add_space(8.0);

        // Local preview, rendered with the same code as the daemon.
        let now = Instant::now();
        let dt = now.duration_since(self.preview_last).as_secs_f64();
        self.preview_last = now;
        let c = self.preview.frame(now.duration_since(self.preview_t0).as_secs_f64(), dt);
        ui.horizontal(|ui| {
            ui.label("Preview");
            let (r, _) = ui.allocate_exact_size(egui::vec2(260.0, 36.0), egui::Sense::hover());
            ui.painter().rect_filled(r, 8.0, c32(c));
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.live, "Live apply");
            if ui.button("Apply").clicked() {
                self.send(Request::Effect { effect: self.spec.clone() });
                self.dirty_since = None;
            }
        });
        if self.live && self.dirty_since.is_some_and(|t| t.elapsed() > Duration::from_millis(150)) {
            self.send(Request::Effect { effect: self.spec.clone() });
            self.dirty_since = None;
        }
    }

    fn hardware_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Keyboard");
        if let Some(st) = self.status.clone() {
            let mut b = st.brightness.unwrap_or(0) as u32;
            if labeled(ui, "Brightness", |ui| ui.add(egui::Slider::new(&mut b, 0..=3)).changed()) {
                self.send(Request::Brightness { level: b as u8 });
            }
            let (mut fps, mut gamma, mut intensity) = (st.fps, st.gamma, st.intensity);
            let mut changed = slider(ui, "Frame rate", &mut fps, 5.0..=60.0, " fps");
            changed |= slider(ui, "Gamma", &mut gamma, 0.5..=3.0, "");
            changed |= slider(ui, "Intensity", &mut intensity, 0.0..=1.0, "");
            if changed {
                self.send(Request::Settings { fps: Some(fps), gamma: Some(gamma), intensity: Some(intensity) });
            }
        }
        ui.separator();
        ui.label("Lit when:");
        let p = &mut self.power;
        let mut ch = ui.checkbox(&mut p.boot, "booting").changed();
        ch |= ui.checkbox(&mut p.awake, "awake").changed();
        ch |= ui.checkbox(&mut p.sleep, "sleeping").changed();
        ch |= ui.checkbox(&mut p.keyboard, "keyboard (lid/kbd active)").changed();
        ui.checkbox(&mut self.power_save, "save to EC");
        if ch {
            let (state, save) = (self.power, self.power_save);
            self.send(Request::Power { state, save });
        }
        ui.separator();
        ui.horizontal(|ui| {
            color_edit(ui, &mut self.notify_color);
            if ui.button("Test notification").clicked() {
                self.send(Request::Notify { color: self.notify_color, count: 3, period: 0.4 });
            }
        });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.last_poll.elapsed() > Duration::from_millis(1000) {
            self.poll();
        }
        egui::TopBottomPanel::top("top").show(ctx, |ui| self.header(ui));
        egui::SidePanel::left("profiles").default_width(190.0).show(ctx, |ui| self.profiles_panel(ui));
        egui::SidePanel::right("hw").default_width(250.0).show(ctx, |ui| self.hardware_panel(ui));
        egui::CentralPanel::default().show(ctx, |ui| self.effect_panel(ui));
        ctx.request_repaint_after(Duration::from_millis(33));
    }
}
