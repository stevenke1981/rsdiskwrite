use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

use crate::disk::{Disk, list_disks, write_image};
use acme_core::Scene;
use acme_platform::{
    Application, FrameContext, PlatformEvent, PlatformKey, Waker, WindowConfig, WindowId,
    run as run_app,
};
use acme_render_wgpu::{Frame, Quad, TextRun, scene_from_frame};
use acme_text::{FontSystem, GlyphAtlas, TextConstraints, TextStyle};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Browse,
    Refresh,
    Previous,
    Next,
    Disk(usize),
    AllowLarge,
    Write,
}

enum Update {
    Progress(u64),
    Finished(Result<u64, String>),
}

struct App {
    fonts: FontSystem,
    atlas: GlyphAtlas,
    image: Option<PathBuf>,
    disks: Vec<Disk>,
    selected: Option<usize>,
    page: usize,
    allow_large: bool,
    writing: bool,
    status: String,
    bytes_written: u64,
    buttons: Vec<(Action, [f32; 4])>,
    pressed: Option<Action>,
    focused: usize,
    sender: Sender<Update>,
    receiver: Receiver<Update>,
    waker: Option<Waker>,
}

impl App {
    fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        let mut app = Self {
            fonts: FontSystem::new(),
            atlas: GlyphAtlas::default(),
            image: None,
            disks: Vec::new(),
            selected: None,
            page: 0,
            allow_large: false,
            writing: false,
            status: String::new(),
            bytes_written: 0,
            buttons: Vec::new(),
            pressed: None,
            focused: 0,
            sender,
            receiver,
            waker: None,
        };
        app.refresh();
        app
    }

    fn refresh(&mut self) {
        match list_disks() {
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
                self.status = format!("Cannot list disks: {error}");
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
                    self.image = Some(path);
                    self.status = "Image selected. Choose the target disk.".into();
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
                self.selected = Some(index);
                self.status =
                    "Target selected. Verify disk number and capacity before writing.".into();
            }
            Action::AllowLarge => self.allow_large = !self.allow_large,
            Action::Write => self.start_write(),
        }
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
        let disk = Disk {
            number: disk.number,
            size: disk.size,
            model: disk.model.clone(),
        };
        let confirmation = format!(
            "ALL DATA on PhysicalDrive{} ({}, {} bytes) will be overwritten and CANNOT BE RECOVERED.\n\nImage: {}\n\nAre you certain this is the correct target?",
            disk.number,
            disk.model,
            disk.size,
            image.display()
        );
        if rfd::MessageDialog::new()
            .set_title("Confirm irreversible disk write")
            .set_description(&confirmation)
            .set_level(rfd::MessageLevel::Warning)
            .set_buttons(rfd::MessageButtons::YesNo)
            .show()
            != rfd::MessageDialogResult::Yes
        {
            return;
        }
        let Some(waker) = self.waker.clone() else {
            self.status = "UI event loop is not ready.".into();
            return;
        };
        let sender = self.sender.clone();
        let progress_sender = sender.clone();
        let progress_waker = waker.clone();
        let allow_large = self.allow_large;
        self.writing = true;
        self.bytes_written = 0;
        self.status = format!(
            "Writing PhysicalDrive{}; do not disconnect it.",
            disk.number
        );
        thread::spawn(move || {
            let last = std::sync::atomic::AtomicU64::new(0);
            let result = write_image(&image, &disk, allow_large, move |bytes| {
                if bytes.saturating_sub(last.load(std::sync::atomic::Ordering::Relaxed))
                    >= 1024 * 1024
                {
                    last.store(bytes, std::sync::atomic::Ordering::Relaxed);
                    let _ = progress_sender.send(Update::Progress(bytes));
                    progress_waker.wake();
                }
            });
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
        let enabled = !self.writing;
        let fill = if !enabled {
            [0.19, 0.21, 0.25, 1.0]
        } else if selected {
            [0.15, 0.35, 0.58, 1.0]
        } else {
            [0.16, 0.19, 0.24, 1.0]
        };
        let focused = self.buttons.len() == self.focused && enabled;
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
                Update::Progress(bytes) => self.bytes_written = bytes,
                Update::Finished(result) => {
                    self.writing = false;
                    match result {
                        Ok(bytes) => {
                            self.bytes_written = bytes;
                            self.status = format!("Write completed: {bytes} bytes.");
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
                self.focused = if shift {
                    (self.focused + self.buttons.len() - 1) % self.buttons.len()
                } else {
                    (self.focused + 1) % self.buttons.len()
                };
                true
            }
            PlatformEvent::Key {
                key: PlatformKey::Enter | PlatformKey::Space,
                pressed: true,
                ..
            } => {
                if let Some((action, _)) = self.buttons.get(self.focused) {
                    self.activate(*action);
                }
                true
            }
            PlatformEvent::FileDropped { paths, .. } if !self.writing => {
                if let Some(path) = paths.into_iter().next() {
                    self.image = Some(path.into());
                    self.status = "Image selected. Choose the target disk.".into();
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
                let model: String = disk.model.chars().take(42).collect();
                let title = format!(
                    "{} PhysicalDrive{}   |   {} bytes ({:.2} GiB)   |   {}",
                    if self.selected == Some(index) {
                        "(*)"
                    } else {
                        "( )"
                    },
                    disk.number,
                    disk.size,
                    disk.size as f64 / 1_073_741_824.0,
                    model
                );
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
        let protection = if self.allow_large {
            "[x] Allow writing disks larger than 2 GiB"
        } else {
            "[ ] Allow writing disks larger than 2 GiB"
        };
        self.button(
            &mut frame,
            Action::AllowLarge,
            [28.0, 465.0, 365.0, 38.0],
            protection,
            self.allow_large,
            scale,
        );
        self.button(
            &mut frame,
            Action::Write,
            [width - 190.0, 465.0, 162.0, 38.0],
            "Write image...",
            true,
            scale,
        );
        self.text(
            &mut frame,
            "WARNING: Writing overwrites data and cannot be undone.",
            28.0,
            525.0,
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
            555.0,
            15.0,
            [0.88, 0.90, 0.93, 1.0],
            scale,
        );
        self.text(
            &mut frame,
            &format!("Written: {} bytes", self.bytes_written),
            28.0,
            583.0,
            15.0,
            [0.70, 0.74, 0.80, 1.0],
            scale,
        );
        scene_from_frame(&frame)
    }
}

pub fn run() -> Result<(), String> {
    run_app(App::new()).map_err(|error| error.to_string())
}
