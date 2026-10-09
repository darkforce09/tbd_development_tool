//! F3 debug panel: project numbers, frame timing, process telemetry, GPU and canvas stats.

use eframe::egui;
use egui::{Color32, FontFamily, FontId, Pos2, RichText, Sense, Stroke, Vec2};
use std::collections::VecDeque;
use std::time::Duration;
use studio_ui::color_tokens::*;

use super::StudioApp;
use crate::telemetry::{HardwareSnapshot, ProcessProbe};

/// Frame times kept for the sparkline.
pub const FRAME_HISTORY: usize = 120;
/// How often process telemetry is sampled while the panel is open.
const SAMPLE_INTERVAL: Duration = Duration::from_millis(500);
const FRAME_BUDGET_MS: f32 = 1000.0 / 60.0;

#[derive(Default)]
pub struct DebugState {
    pub open: bool,
    pub frame_times_ms: VecDeque<f32>,
    probe: ProcessProbe,
    snapshot: Option<HardwareSnapshot>,
    /// Process CPU use over the last sample interval, in percent of one core.
    cpu_percent: Option<f64>,
}

impl DebugState {
    /// Records one frame's duration, keeping the last `FRAME_HISTORY` frames.
    pub fn push_frame_time(&mut self, ms: f32) {
        if self.frame_times_ms.len() == FRAME_HISTORY {
            self.frame_times_ms.pop_front();
        }
        self.frame_times_ms.push_back(ms);
    }

    /// Samples process telemetry if the last sample is older than `SAMPLE_INTERVAL`.
    fn sample(&mut self) {
        let due = self.snapshot.is_none_or(|s| s.timestamp.elapsed() >= SAMPLE_INTERVAL);
        if !due {
            return;
        }
        let now = self.probe.capture();
        if let Some(prev) = self.snapshot {
            let wall_ms = now.timestamp.duration_since(prev.timestamp).as_secs_f64() * 1000.0;
            if wall_ms > 0.0 {
                self.cpu_percent = Some((now.cpu_time_ms - prev.cpu_time_ms).max(0.0) / wall_ms * 100.0);
            }
        }
        self.snapshot = Some(now);
    }
}

fn fmt_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(title).font(FontId::new(10.5, FontFamily::Monospace)).color(TEXT_DIM).strong());
}

fn rows(ui: &mut egui::Ui, id: &str, entries: &[(&str, String)]) {
    egui::Grid::new(id).num_columns(2).spacing([16.0, 2.0]).show(ui, |ui| {
        for (label, value) in entries {
            ui.label(RichText::new(*label).font(FontId::new(11.0, FontFamily::Proportional)).color(TEXT_SECONDARY));
            ui.label(RichText::new(value).font(FontId::new(11.0, FontFamily::Monospace)).color(TEXT_PRIMARY));
            ui.end_row();
        }
    });
}

/// Frame-time sparkline with the 60 FPS budget as a reference line.
fn frame_sparkline(ui: &mut egui::Ui, frame_times_ms: &VecDeque<f32>) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(280.0, 44.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, Color32::from_black_alpha(60));
    let max_ms = frame_times_ms.iter().copied().fold(FRAME_BUDGET_MS * 2.0, f32::max);
    let y_for = |ms: f32| rect.bottom() - (ms / max_ms).min(1.0) * rect.height();

    let budget_y = y_for(FRAME_BUDGET_MS);
    painter.hline(rect.x_range(), budget_y, Stroke::new(1.0, Color32::from_rgba_unmultiplied(250, 204, 21, 90)));

    if frame_times_ms.len() >= 2 {
        let step = rect.width() / (FRAME_HISTORY - 1) as f32;
        let offset = FRAME_HISTORY - frame_times_ms.len();
        let points: Vec<Pos2> = frame_times_ms
            .iter()
            .enumerate()
            .map(|(i, &ms)| Pos2::new(rect.left() + (i + offset) as f32 * step, y_for(ms)))
            .collect();
        painter.add(egui::Shape::line(points, Stroke::new(1.2, ARCHETYPE_EGRESS)));
    }
}

impl StudioApp {
    pub(crate) fn render_debug_panel(&mut self, ctx: &egui::Context) {
        if !self.debug.open {
            return;
        }
        self.debug.sample();

        let mut open = true;
        egui::Window::new(format!("{} Debug", egui_phosphor::regular::BUG))
            .id(egui::Id::new("debug_panel"))
            .open(&mut open)
            .resizable(false)
            .collapsible(true)
            .default_pos(Pos2::new(ctx.viewport_rect().right() - 340.0, 60.0))
            .show(ctx, |ui| {
                ui.set_width(300.0);
                self.debug_project(ui);
                self.debug_frame(ui);
                self.debug_process(ui);
                self.debug_gpu(ui);
                self.debug_canvas(ui);
            });
        self.debug.open = open;
    }

    fn debug_project(&self, ui: &mut egui::Ui) {
        section(ui, "PROJECT");
        if let Some(err) = &self.load_error {
            ui.label(
                RichText::new(format!("{} Load failed: {err}", egui_phosphor::regular::WARNING)).color(ARCHETYPE_STATE),
            );
        }
        let Some(stats) = &self.project_stats else {
            ui.label(RichText::new("No project open").color(TEXT_SECONDARY));
            return;
        };
        if let Some(path) = &self.current_project_path {
            ui.label(
                RichText::new(path.display().to_string())
                    .font(FontId::new(10.0, FontFamily::Monospace))
                    .color(TEXT_DIM),
            );
        }
        rows(
            ui,
            "debug_project",
            &[
                ("Name", stats.project_name.clone()),
                ("Files", stats.file_count.to_string()),
                ("Crates", stats.crate_count.to_string()),
                ("Nodes", self.graph.nodes.len().to_string()),
                ("Wires", self.graph.edges.len().to_string()),
                ("Folders", self.graph.clusters.len().to_string()),
                ("Nodes without wires", self.graph.isolated_nodes_count().to_string()),
                ("Loaded from cache", if self.is_from_cache { "yes" } else { "no" }.to_string()),
            ],
        );
        if let Some(msg) = &self.canvas_state.status_message {
            ui.label(RichText::new(msg).font(FontId::new(10.5, FontFamily::Proportional)).color(TEXT_DIM));
        }
    }

    fn debug_frame(&self, ui: &mut egui::Ui) {
        section(ui, "FRAME");
        let last = self.debug.frame_times_ms.back().copied().unwrap_or(0.0);
        let worst = self.debug.frame_times_ms.iter().copied().fold(0.0, f32::max);
        rows(
            ui,
            "debug_frame",
            &[
                ("FPS", format!("{:.0}", self.fps)),
                ("Frame time", format!("{last:.2} ms")),
                ("Worst of last 120", format!("{worst:.2} ms")),
            ],
        );
        frame_sparkline(ui, &self.debug.frame_times_ms);
    }

    fn debug_process(&self, ui: &mut egui::Ui) {
        section(ui, "PROCESS");
        let Some(s) = self.debug.snapshot else {
            ui.label(RichText::new("Sampling…").color(TEXT_DIM));
            return;
        };
        let opt_bytes = |v: Option<u64>| v.map_or("n/a".to_string(), fmt_bytes);
        let opt_num = |v: Option<u64>| v.map_or("n/a".to_string(), |n| n.to_string());
        rows(
            ui,
            "debug_process",
            &[
                ("Memory (RSS)", fmt_bytes(s.rss_bytes)),
                ("Peak RSS", opt_bytes(s.os_peak_rss_bytes)),
                ("Heap (anon RSS)", opt_bytes(s.rss_anon_bytes)),
                ("Mapped files (RSS)", opt_bytes(s.rss_file_bytes)),
                ("Virtual memory", fmt_bytes(s.virtual_bytes)),
                ("CPU", self.debug.cpu_percent.map_or("n/a".to_string(), |p| format!("{p:.0}%"))),
                ("Threads", s.thread_count.map_or("n/a".to_string(), |t| t.to_string())),
                ("Disk read", fmt_bytes(s.disk_read_bytes)),
                ("Disk written", fmt_bytes(s.disk_written_bytes)),
                ("Page faults (minor)", opt_num(s.minor_faults)),
                ("Page faults (major)", opt_num(s.major_faults)),
            ],
        );
    }

    fn debug_gpu(&self, ui: &mut egui::Ui) {
        section(ui, "GPU");
        let wires = if self.canvas_state.use_gpu_wires { "GPU" } else { "CPU" }.to_string();
        match &self.gpu {
            Some(gpu) => rows(
                ui,
                "debug_gpu",
                &[
                    ("Adapter", gpu.name.clone()),
                    ("Backend", gpu.backend.clone()),
                    ("Type", gpu.device_type.clone()),
                    ("Driver", gpu.driver.clone()),
                    ("Wire rendering", wires),
                ],
            ),
            None => {
                rows(ui, "debug_gpu", &[("Adapter", "none (CPU rendering)".to_string()), ("Wire rendering", wires)])
            }
        }
    }

    fn debug_canvas(&self, ui: &mut egui::Ui) {
        section(ui, "CANVAS");
        let stats = self.canvas_state.frame_stats;
        rows(
            ui,
            "debug_canvas",
            &[
                ("Zoom", format!("{:.1}%", self.canvas_state.transform.zoom * 100.0)),
                ("Visible nodes", stats.visible_nodes.to_string()),
                ("Visible wires", stats.visible_wires.to_string()),
            ],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_history_is_capped() {
        let mut debug = DebugState::default();
        for i in 0..(FRAME_HISTORY + 30) {
            debug.push_frame_time(i as f32);
        }
        assert_eq!(debug.frame_times_ms.len(), FRAME_HISTORY);
        assert_eq!(debug.frame_times_ms.front().copied(), Some(30.0));
    }

    #[test]
    fn bytes_format_with_units() {
        assert_eq!(fmt_bytes(512), "512 B");
        assert_eq!(fmt_bytes(1536), "1.5 KB");
        assert_eq!(fmt_bytes(3 * 1024 * 1024), "3.0 MB");
    }
}
