//! The graphical interface (AcmeUI Native).

use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

use crate::disk::{self, Disk};
use crate::error::Result;
use crate::ops::{self, Phase, Progress, WritePlan, WriteReport, WriteRequest};
use crate::output::format_size;
use acme_core::Scene;
use acme_platform::{
    Application, FrameContext, PlatformEvent, PlatformKey, Waker, WindowConfig, WindowId,
    run as run_app,
};
use acme_render_wgpu::{Frame, Quad, TextRun, scene_from_frame};
use acme_text::{FontSystem, GlyphAtlas, TextConstraints, TextStyle};
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Browse,
    Refresh,
    Previous,
    Next,
    Disk(usize),
    AllowLarge,
    Verify,
    AllowFixed,
    Write,
}

/// Outcome of a finished write, ready for display.
struct Done {
    message: String,
    /// False when the write succeeded but read-back verification failed.
    ok: bool,
    bytes: u64,
}

enum Update {
    Progress(Progress),
    Finished(std::result::Result<Done, String>),
}

/// One-line row label: selection mark, number, bus, flags, size, model, serial.
fn disk_row_label(disk: &Disk, selected: bool) -> String {
    let mark = if disk.is_system {
        "[X]"
    } else if selected {
        "(*)"
    } else {
        "( )"
    };
    let mut flags = String::new();
    if disk.removable {
        flags.push_str(" removable");
    }
    if disk.is_system {
        flags.push_str(" SYSTEM");
    } else if !disk.is_removable_target() {
        flags.push_str(" fixed");
    }
    let serial = disk
        .serial
        .as_deref()
        .map(|s| format!("  SN {s}"))
        .unwrap_or_default();
    format!(
        "{mark} #{}  {}{}  {}  {}{serial}",
        disk.number,
        disk.bus.label(),
        flags,
        format_size(disk.size),
        disk.model
    )
}

/// Turns a plan into "go ahead" or the text listing every refusal.
fn evaluate_plan(plan: WritePlan) -> std::result::Result<WritePlan, String> {
    if plan.is_allowed() {
        return Ok(plan);
    }
    let lines: Vec<String> = plan
        .refusals
        .iter()
        .map(|r| match &r.hint {
            Some(hint) => format!("- {} ({hint})", r.message),
            None => format!("- {}", r.message),
        })
        .collect();
    Err(lines.join("\n"))
}

fn short_hash(digest: &crate::hash::Digest) -> String {
    digest.to_hex().chars().take(12).collect()
}

fn summarize(report: &WriteReport) -> Done {
    let mut message = format!("Write completed: {} bytes", report.bytes_written);
    let mut ok = true;
    match &report.verify {
        Some(v) if v.matched => message.push_str(", verify OK"),
        Some(v) => {
            ok = false;
            message = format!(
                "Write finished but VERIFY FAILED (first difference at byte {}).",
                v.first_mismatch.unwrap_or(0)
            );
        }
        None => message.push_str(", not verified"),
    }
    if let Some(hash) = report.content_sha256.as_ref().or(report
        .verify
        .as_ref()
        .and_then(|v| v.content_sha256.as_ref()))
    {
        message.push_str(&format!(", SHA-256 {}...", short_hash(hash)));
    }
    Done {
        message,
        ok,
        bytes: report.bytes_written,
    }
}

fn phase_label(phase: Phase) -> &'static str {
    match phase {
        Phase::Preparing => "Preparing",
        Phase::Hashing => "Hashing",
        Phase::Writing => "Writing",
        Phase::Flushing => "Flushing",
        Phase::Verifying => "Verifying",
    }
}

struct App {
    fonts: FontSystem,
    atlas: GlyphAtlas,
    image: Option<PathBuf>,
    disks: Vec<Disk>,
    selected: Option<usize>,
    page: usize,
    allow_large: bool,
    verify: bool,
    allow_fixed: bool,
    writing: bool,
    progress: Option<Progress>,
    phase_started: Instant,
    rate: f64,
    status: String,
    bytes_written: u64,
    buttons: Vec<(Action, [f32; 4])>,
    pressed: Option<Action>,
    focused: Action,
    sender: Sender<Update>,
    receiver: Receiver<Update>,
    waker: Option<Waker>,
}

impl App {
    fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            fonts: FontSystem::new(),
            atlas: GlyphAtlas::default(),
            image: None,
            disks: Vec::new(),
            selected: None,
            page: 0,
            allow_large: false,
            verify: true,
            allow_fixed: false,
            writing: false,
            progress: None,
            phase_started: Instant::now(),
            rate: 0.0,
            status: String::new(),
            bytes_written: 0,
            buttons: Vec::new(),
            pressed: None,
            focused: Action::Browse,
            sender,
            receiver,
            waker: None,
        }
    }

    fn refresh(&mut self) {
        match disk::list() {
            Ok(disks) => {
                self.status = if disks.is_empty() {
                    "No disks found. Run as Administrator and check device connections.".into()
                } else {
                    format!(
                        "Found {} physical disk(s). Choose a target carefully.",
                        disks.len()
                    )
                };
                self.disks = disks;
                self.selected = None;
                self.page = 0;
            }
            Err(error) => {
                self.disks.clear();
                self.selected = None;
                self.status = format!("Cannot list disks: {}", error.message);
            }
        }
    }

    fn activate(&mut self, action: Action) {
        if self.writing {
            return;
        }
        match action {
            Action::Browse => {
                if let Some(path) = rfd::FileDialog::new()
                    .set_title("Select disk image")
                    .pick_file()
                {
                    self.select_image(path);
                }
            }
            Action::Refresh => self.refresh(),
            Action::Previous => self.page = self.page.saturating_sub(1),
            Action::Next => {
                if (self.page + 1) * 4 < self.disks.len() {
                    self.page += 1;
                }
            }
            Action::Disk(index) => {
                if self.disks.get(index).is_none_or(|d| d.is_system) {
                    self.status = "System disks cannot be selected.".into();
                    return;
                }
                self.selected = Some(index);
                self.status =
                    "Target selected. Verify disk number and capacity before writing.".into();
            }
            Action::AllowLarge => self.allow_large = !self.allow_large,
            Action::Verify => self.verify = !self.verify,
            Action::AllowFixed => self.allow_fixed = !self.allow_fixed,
            Action::Write => self.start_write(),
        }
    }

    fn select_image(&mut self, path: PathBuf) {
        self.image = Some(path);
        self.selected = None;
        self.bytes_written = 0;
        self.status = "Image selected. Choose the target disk.".into();
    }

    fn start_write(&mut self) {
        let Some(image) = self.image.clone() else {
            self.status = "Select an image first.".into();
            return;
        };
        let Some(disk) = self.selected.and_then(|index| self.disks.get(index)) else {
            self.status = "Select a target disk first.".into();
            return;
        };
        let request = WriteRequest {
            expected: Some(disk.identity()),
            allow_large: self.allow_large,
            allow_fixed: self.allow_fixed,
            verify: self.verify,
            ..WriteRequest::new(&image, disk.number)
        };
        let plan = match ops::plan(&request)
            .map_err(|e| e.message)
            .and_then(evaluate_plan)
        {
            Ok(plan) => plan,
            Err(text) => {
                self.status = "Write refused. See the dialog for the reasons.".into();
                rfd::MessageDialog::new()
                    .set_title("Write refused")
                    .set_description(&text)
                    .set_level(rfd::MessageLevel::Error)
                    .set_buttons(rfd::MessageButtons::Ok)
                    .show();
                return;
            }
        };
        let disk = plan.disk.clone();
        let confirmation = format!(
            "ALL DATA on this disk will be overwritten and CANNOT BE RECOVERED.\n\nDisk number: {}\nModel: {}\nSerial: {}\nBus: {}\nSize: {} ({} bytes)\nImage: {}\nVerify after write: {}\n\nAre you certain this is the correct target?\n\n(Default: No. Pressing Enter or closing this window cancels.)",
            disk.number,
            disk.model,
            disk.serial.as_deref().unwrap_or("(none)"),
            disk.bus.label(),
            format_size(disk.size),
            disk.size,
            image.display(),
            if self.verify { "yes" } else { "no" }
        );
        // MessageBoxW, not rfd: rfd cannot make No the default button, and a default Yes
        // would let a stray Enter (or a held, auto-repeating key) erase the disk.
        if !crate::mcp::gate::dialog::yes_no_default_no(
            "Confirm irreversible disk write",
            &confirmation,
        ) {
            self.status = "Write cancelled; nothing was written.".into();
            return;
        }
        let Some(waker) = self.waker.clone() else {
            self.status = "UI event loop is not ready.".into();
            return;
        };
        let sender = self.sender.clone();
        let progress_sender = sender.clone();
        let progress_waker = waker.clone();
        self.writing = true;
        self.bytes_written = 0;
        self.progress = None;
        self.rate = 0.0;
        self.phase_started = Instant::now();
        self.status = format!(
            "Writing PhysicalDrive{}; do not disconnect it.",
            disk.number
        );
        thread::spawn(move || {
            let last = std::sync::atomic::AtomicU64::new(0);
            let last_phase = std::sync::Mutex::new(None::<Phase>);
            let result = ops::execute(&plan, &|progress| {
                let bytes = progress.done;
                let changed = {
                    let mut guard = last_phase.lock().unwrap_or_else(|e| e.into_inner());
                    let changed = *guard != Some(progress.phase);
                    *guard = Some(progress.phase);
                    changed
                };
                if changed {
                    last.store(bytes, std::sync::atomic::Ordering::Relaxed);
                }
                if changed
                    || bytes.saturating_sub(last.load(std::sync::atomic::Ordering::Relaxed))
                        >= 1024 * 1024
                {
                    last.store(bytes, std::sync::atomic::Ordering::Relaxed);
                    let _ = progress_sender.send(Update::Progress(progress));
                    progress_waker.wake();
                }
            })
            .map(|report| summarize(&report))
            .map_err(|error| error.message);
            let _ = sender.send(Update::Finished(result));
            waker.wake();
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn text(
        &mut self,
        frame: &mut Frame,
        content: &str,
        x: f32,
        y: f32,
        size: f32,
        color: [f32; 4],
        scale: f32,
    ) {
        let style = TextStyle {
            font_size: size,
            line_height: size * 1.35,
            ..TextStyle::default()
        };
        let shaped = self
            .fonts
            .shape(content, &style, TextConstraints::default(), scale);
        let prepared = self.fonts.prepare(&shaped, &mut self.atlas);
        frame.text.push(TextRun {
            prepared,
            origin: [x, y + size],
            color,
            clip: None,
        });
    }

    fn button(
        &mut self,
        frame: &mut Frame,
        action: Action,
        rect: [f32; 4],
        text: &str,
        selected: bool,
        scale: f32,
    ) {
        let enabled = !self.writing
            && !matches!(action, Action::Disk(i) if self.disks.get(i).is_none_or(|d| d.is_system));
        let fill = if !enabled {
            [0.19, 0.21, 0.25, 1.0]
        } else if selected {
            [0.15, 0.35, 0.58, 1.0]
        } else {
            [0.16, 0.19, 0.24, 1.0]
        };
        let focused = action == self.focused && enabled;
        frame.quads.push(Quad {
            rect,
            color: fill,
            radius: 6.0,
            border_width: if focused { 2.0 } else { 1.0 },
            border_color: if focused {
                [0.45, 0.72, 1.0, 1.0]
            } else {
                [0.30, 0.34, 0.40, 1.0]
            },
        });
        self.text(
            frame,
            text,
            rect[0] + 10.0,
            rect[1] + 7.0,
            15.0,
            [0.91, 0.93, 0.96, 1.0],
            scale,
        );
        self.buttons.push((action, rect));
    }
}

impl Application for App {
    fn window_config(&self) -> WindowConfig {
        WindowConfig {
            title: "rsdiskwrite".into(),
            width: 900.0,
            height: 680.0,
            ..Default::default()
        }
    }
    fn minimum_window_size(&self) -> Option<(f64, f64)> {
        Some((760.0, 660.0))
    }
    fn on_waker(&mut self, waker: Waker) {
        self.waker = Some(waker);
    }
    fn wake(&mut self) -> bool {
        let mut changed = false;
        while let Ok(update) = self.receiver.try_recv() {
            changed = true;
            match update {
                Update::Progress(progress) => {
                    if self.progress.map(|p| p.phase) != Some(progress.phase) {
                        self.phase_started = Instant::now();
                    }
                    let secs = self.phase_started.elapsed().as_secs_f64();
                    if secs > 0.2 {
                        self.rate = progress.done as f64 / secs;
                    }
                    if progress.phase == Phase::Writing {
                        self.bytes_written = progress.done;
                    }
                    self.progress = Some(progress);
                }
                Update::Finished(result) => {
                    self.writing = false;
                    self.progress = None;
                    match result {
                        Ok(done) => {
                            self.bytes_written = done.bytes;
                            self.status = done.message.clone();
                            if !done.ok {
                                rfd::MessageDialog::new()
                                    .set_title("Verification failed")
                                    .set_description(&done.message)
                                    .set_level(rfd::MessageLevel::Error)
                                    .set_buttons(rfd::MessageButtons::Ok)
                                    .show();
                            }
                        }
                        Err(error) => {
                            self.status = format!("Write failed: {error}");
                            rfd::MessageDialog::new()
                                .set_title("Write failed")
                                .set_description(&error)
                                .set_level(rfd::MessageLevel::Error)
                                .set_buttons(rfd::MessageButtons::Ok)
                                .show();
                        }
                    }
                }
            }
        }
        changed
    }
    fn close_requested(&mut self, _window: WindowId) -> bool {
        !self.writing
    }
    fn on_gpu_recovered(&mut self, _window: WindowId) {
        self.atlas.clear();
    }
    fn event(&mut self, event: PlatformEvent) -> bool {
        match event {
            PlatformEvent::PointerButton {
                x,
                y,
                pressed,
                button: 0,
                ..
            } => {
                let hit = self
                    .buttons
                    .iter()
                    .find(|(_, r)| x >= r[0] && y >= r[1] && x < r[0] + r[2] && y < r[1] + r[3])
                    .map(|(a, _)| *a);
                if pressed {
                    self.pressed = hit;
                    if let Some(action) = hit {
                        self.focused = action;
                    }
                } else {
                    let down = self.pressed.take();
                    if let Some(action) = hit.filter(|_| hit == down) {
                        self.activate(action);
                    }
                }
                true
            }
            PlatformEvent::Key {
                key: PlatformKey::Tab,
                pressed: true,
                shift,
                ..
            } if !self.buttons.is_empty() => {
                let len = self.buttons.len();
                let index = match self
                    .buttons
                    .iter()
                    .position(|(action, _)| *action == self.focused)
                {
                    Some(index) if shift => (index + len - 1) % len,
                    Some(index) => (index + 1) % len,
                    None if shift => len - 1,
                    None => 0,
                };
                self.focused = self.buttons[index].0;
                true
            }
            // Auto-repeat from a held key never activates anything, so holding Enter cannot
            // click through a dialog into the next action.
            PlatformEvent::Key {
                key: PlatformKey::Enter | PlatformKey::Space,
                pressed: true,
                repeat,
                ..
            } => {
                if repeat {
                    return false;
                }
                if self
                    .buttons
                    .iter()
                    .any(|(action, _)| *action == self.focused)
                {
                    self.activate(self.focused);
                }
                true
            }
            PlatformEvent::FileDropped { paths, .. } if !self.writing => {
                if let Some(path) = paths.into_iter().next() {
                    self.select_image(path.into());
                }
                true
            }
            PlatformEvent::Resized { .. } => true,
            _ => false,
        }
    }
    fn frame(&mut self, context: FrameContext) -> Scene {
        let width = context.logical_width;
        let scale = context.scale_factor;
        let mut frame = Frame {
            clear: [0.075, 0.085, 0.105, 1.0],
            ..Default::default()
        };
        self.buttons.clear();
        self.text(
            &mut frame,
            "rsdiskwrite",
            28.0,
            20.0,
            28.0,
            [0.96, 0.97, 0.99, 1.0],
            scale,
        );
        self.text(
            &mut frame,
            "Write a disk image to a physical drive",
            28.0,
            58.0,
            15.0,
            [0.70, 0.74, 0.80, 1.0],
            scale,
        );
        self.text(
            &mut frame,
            "IMAGE",
            28.0,
            101.0,
            14.0,
            [0.70, 0.74, 0.80, 1.0],
            scale,
        );
        let image = self
            .image
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "No image selected (or drop a file here)".into());
        let image_display: String = image
            .chars()
            .take(((width - 180.0) / 8.0).max(20.0) as usize)
            .collect();
        self.text(
            &mut frame,
            &image_display,
            28.0,
            128.0,
            15.0,
            [0.92, 0.93, 0.95, 1.0],
            scale,
        );
        self.button(
            &mut frame,
            Action::Browse,
            [width - 140.0, 119.0, 112.0, 38.0],
            "Browse...",
            false,
            scale,
        );
        self.text(
            &mut frame,
            "PHYSICAL DISKS",
            28.0,
            183.0,
            14.0,
            [0.70, 0.74, 0.80, 1.0],
            scale,
        );
        self.button(
            &mut frame,
            Action::Refresh,
            [width - 140.0, 174.0, 112.0, 38.0],
            "Refresh",
            false,
            scale,
        );
        for row in 0..4 {
            let index = self.page * 4 + row;
            if let Some(disk) = self.disks.get(index) {
                let label: String = disk_row_label(disk, self.selected == Some(index))
                    .chars()
                    .take(((width - 80.0) / 8.0).max(30.0) as usize)
                    .collect();
                let title = label;
                self.button(
                    &mut frame,
                    Action::Disk(index),
                    [28.0, 222.0 + row as f32 * 45.0, width - 56.0, 39.0],
                    &title,
                    self.selected == Some(index),
                    scale,
                );
            }
        }
        self.button(
            &mut frame,
            Action::Previous,
            [28.0, 410.0, 100.0, 38.0],
            "Previous",
            false,
            scale,
        );
        self.text(
            &mut frame,
            &format!(
                "Page {} / {}",
                self.page + 1,
                self.disks.len().max(1).div_ceil(4)
            ),
            145.0,
            417.0,
            15.0,
            [0.80, 0.83, 0.88, 1.0],
            scale,
        );
        self.button(
            &mut frame,
            Action::Next,
            [250.0, 410.0, 100.0, 38.0],
            "Next",
            false,
            scale,
        );
        let check = |on: bool, text: &str| format!("{} {text}", if on { "[x]" } else { "[ ]" });
        let toggles = [
            (
                Action::AllowLarge,
                check(self.allow_large, "Allow disks larger than 2 GiB"),
                self.allow_large,
                [28.0, 458.0, 330.0, 36.0],
            ),
            (
                Action::Verify,
                check(self.verify, "Verify after write"),
                self.verify,
                [372.0, 458.0, 250.0, 36.0],
            ),
            (
                Action::AllowFixed,
                check(self.allow_fixed, "Allow fixed (non-removable) disks"),
                self.allow_fixed,
                [28.0, 502.0, 330.0, 36.0],
            ),
        ];
        for (action, label, on, rect) in toggles {
            self.button(&mut frame, action, rect, &label, on, scale);
        }
        let write_label = match self.selected.and_then(|index| self.disks.get(index)) {
            Some(disk) => format!("Write to disk {}...", disk.number),
            None => "Write image...".into(),
        };
        self.button(
            &mut frame,
            Action::Write,
            [width - 190.0, 502.0, 162.0, 36.0],
            &write_label,
            true,
            scale,
        );
        self.text(
            &mut frame,
            "WARNING: Writing overwrites data and cannot be undone.",
            28.0,
            550.0,
            15.0,
            [1.0, 0.72, 0.48, 1.0],
            scale,
        );
        let status: String = self
            .status
            .chars()
            .take(((width - 56.0) / 8.0).max(30.0) as usize)
            .collect();
        self.text(
            &mut frame,
            &status,
            28.0,
            578.0,
            15.0,
            [0.88, 0.90, 0.93, 1.0],
            scale,
        );
        let progress_line = match self.progress {
            Some(p) if self.writing => {
                let pct = if p.total > 0 {
                    (p.done as f64 / p.total as f64 * 100.0).min(100.0)
                } else {
                    0.0
                };
                format!(
                    "{}: {pct:.0}%  ({} / {})  {:.1} MiB/s",
                    phase_label(p.phase),
                    format_size(p.done),
                    format_size(p.total),
                    self.rate / 1_048_576.0
                )
            }
            _ => format!("Written: {} bytes", self.bytes_written),
        };
        self.text(
            &mut frame,
            &progress_line,
            28.0,
            606.0,
            15.0,
            [0.70, 0.74, 0.80, 1.0],
            scale,
        );
        scene_from_frame(&frame)
    }
}

/// Opens the main window and blocks until it is closed.
pub fn run() -> Result<()> {
    let mut app = App::new();
    app.refresh();
    run_app(app).map_err(|error| crate::Error::internal(format!("GUI failed: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_navigation_preserves_focus_and_a_new_image_clears_the_target() {
        let mut app = App::new();
        app.disks = (0..5).map(crate::disk::model::test_disk).collect();
        app.selected = Some(0);
        app.bytes_written = 512;
        app.focused = Action::Next;
        let frame = FrameContext {
            window: WindowId(1),
            logical_width: 900.0,
            logical_height: 680.0,
            scale_factor: 1.0,
        };
        let key = |key| PlatformEvent::Key {
            window: WindowId(1),
            key,
            pressed: true,
            shift: false,
            ctrl: false,
            alt: false,
            meta: false,
            repeat: false,
            text: None,
            character: None,
        };

        app.frame(frame.clone());
        assert!(app.event(key(PlatformKey::Enter)));
        assert_eq!(app.page, 1);
        app.frame(frame);
        assert!(app.event(key(PlatformKey::Tab)));
        assert!(app.focused == Action::AllowLarge);

        assert!(app.event(PlatformEvent::FileDropped {
            window: WindowId(1),
            paths: vec!["second.img".into()]
        }));
        assert_eq!(app.image, Some(PathBuf::from("second.img")));
        assert_eq!(app.selected, None);
        assert_eq!(app.bytes_written, 0);
    }

    #[test]
    fn auto_repeated_keys_do_not_activate() {
        let mut app = App::new();
        app.disks = (0..5).map(crate::disk::model::test_disk).collect();
        app.focused = Action::Next;
        app.frame(FrameContext {
            window: WindowId(1),
            logical_width: 900.0,
            logical_height: 680.0,
            scale_factor: 1.0,
        });
        for key in [PlatformKey::Enter, PlatformKey::Space] {
            let held = PlatformEvent::Key {
                window: WindowId(1),
                key,
                pressed: true,
                shift: false,
                ctrl: false,
                alt: false,
                meta: false,
                repeat: true,
                text: None,
                character: None,
            };
            assert!(!app.event(held));
        }
        assert_eq!(
            app.page, 0,
            "a repeated key must not activate the focused button"
        );
    }

    #[test]
    fn system_disk_cannot_be_selected() {
        let mut app = App::new();
        let mut system = crate::disk::model::test_disk(0);
        system.is_system = true;
        app.disks = vec![system, crate::disk::model::test_disk(1)];
        app.activate(Action::Disk(0));
        assert_eq!(app.selected, None);
        assert!(disk_row_label(&app.disks[0], false).contains("SYSTEM"));
        app.activate(Action::Disk(1));
        assert_eq!(app.selected, Some(1));
    }

    #[test]
    fn toggles_flip_and_have_safe_defaults() {
        let mut app = App::new();
        assert!(app.verify);
        assert!(!app.allow_fixed);
        assert!(!app.allow_large);
        app.activate(Action::Verify);
        app.activate(Action::AllowFixed);
        app.activate(Action::AllowLarge);
        assert!(!app.verify && app.allow_fixed && app.allow_large);
        app.activate(Action::Verify);
        assert!(app.verify);
    }

    #[test]
    fn refusals_block_the_write() {
        use crate::ops::{PlannedImage, Refusal, RefusalCode};
        let plan = WritePlan {
            request: WriteRequest::new("x.img", 1),
            disk: crate::disk::model::test_disk(1),
            image: PlannedImage {
                path: "x.img".into(),
                format: crate::image::ImageFormat::Raw,
                file_size: 512,
                sha256: None,
            },
            refusals: vec![Refusal {
                code: RefusalCode::FixedDisk,
                message: "disk is fixed".into(),
                hint: Some("use allow fixed".into()),
            }],
            warnings: vec![],
        };
        let text = evaluate_plan(plan.clone()).expect_err("refused");
        assert!(text.contains("disk is fixed") && text.contains("use allow fixed"));
        let mut ok = plan;
        ok.refusals.clear();
        assert!(evaluate_plan(ok).is_ok());
    }
}
