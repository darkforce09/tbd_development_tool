use std::time::{Duration, Instant};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

/// Performance budget the benchmark is checked against.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TelemetryBudget {
    pub ram_bytes: u64,
    pub target_fps: f64,
}

impl TelemetryBudget {
    pub const DEFAULT_TARGET_FPS: f64 = 60.0;

    /// Half of system RAM and 60 FPS.
    pub fn for_this_machine() -> Self {
        let mut sys = System::new();
        sys.refresh_memory();
        Self { ram_bytes: sys.total_memory() / 2, target_fps: Self::DEFAULT_TARGET_FPS }
    }

    /// Frame time budget in microseconds.
    pub fn frame_budget_us(&self) -> f64 {
        1_000_000.0 / self.target_fps
    }

    pub fn ram_gb(&self) -> f64 {
        self.ram_bytes as f64 / 1_073_741_824.0
    }

    pub fn describe(&self) -> String {
        format!(
            "RAM ≤ {:.2} GB | {:.0} FPS (≤ {:.2} ms / {:.0} µs per frame)",
            self.ram_gb(),
            self.target_fps,
            self.frame_budget_us() / 1000.0,
            self.frame_budget_us()
        )
    }
}

/// GPU adapter as reported by wgpu.
#[derive(Debug, Clone)]
pub struct GpuDeviceInfo {
    pub name: String,
    pub backend: String,
    pub device_type: String,
    pub driver: String,
}

impl GpuDeviceInfo {
    pub fn from_adapter_info(info: &wgpu::AdapterInfo) -> Self {
        let driver = [info.driver.as_str(), info.driver_info.as_str()]
            .iter()
            .filter(|s| !s.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(" ");
        Self {
            name: info.name.clone(),
            backend: format!("{:?}", info.backend),
            device_type: format!("{:?}", info.device_type),
            driver: if driver.is_empty() { "unknown driver".to_string() } else { driver },
        }
    }

    /// Asks wgpu for the default high-performance adapter without opening a window.
    pub fn detect_headless() -> Option<Self> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .ok()?;
        Some(Self::from_adapter_info(&adapter.get_info()))
    }

    pub fn summary(&self) -> String {
        format!("{} ({} {}, {})", self.name, self.backend, self.device_type, self.driver)
    }
}

/// Process resource usage at one instant. Fields the OS cannot report are `None`.
#[derive(Debug, Clone, Copy)]
pub struct HardwareSnapshot {
    pub timestamp: Instant,
    pub rss_bytes: u64,
    pub virtual_bytes: u64,
    /// Peak resident memory reported by the OS (Linux VmHWM, Unix ru_maxrss).
    pub os_peak_rss_bytes: Option<u64>,
    /// Linux only: anonymous (heap) and file-backed (mmap) resident memory.
    pub rss_anon_bytes: Option<u64>,
    pub rss_file_bytes: Option<u64>,
    /// Total CPU time used by the process so far.
    pub cpu_time_ms: f64,
    /// Unix only: user/system split of `cpu_time_ms`.
    pub user_cpu_ms: Option<f64>,
    pub sys_cpu_ms: Option<f64>,
    pub thread_count: Option<usize>,
    pub disk_read_bytes: u64,
    pub disk_written_bytes: u64,
    /// Unix only.
    pub minor_faults: Option<u64>,
    pub major_faults: Option<u64>,
}

/// Captures snapshots of the current process. Keeps one `System` so refreshes stay cheap.
pub struct ProcessProbe {
    sys: System,
    pid: Option<Pid>,
}

impl Default for ProcessProbe {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessProbe {
    pub fn new() -> Self {
        Self { sys: System::new(), pid: sysinfo::get_current_pid().ok() }
    }

    pub fn capture(&mut self) -> HardwareSnapshot {
        let timestamp = Instant::now();
        let mut snap = HardwareSnapshot {
            timestamp,
            rss_bytes: 0,
            virtual_bytes: 0,
            os_peak_rss_bytes: None,
            rss_anon_bytes: None,
            rss_file_bytes: None,
            cpu_time_ms: 0.0,
            user_cpu_ms: None,
            sys_cpu_ms: None,
            thread_count: None,
            disk_read_bytes: 0,
            disk_written_bytes: 0,
            minor_faults: None,
            major_faults: None,
        };

        if let Some(pid) = self.pid {
            self.sys.refresh_processes_specifics(
                ProcessesToUpdate::Some(&[pid]),
                false,
                ProcessRefreshKind::nothing().with_memory().with_cpu().with_disk_usage(),
            );
            if let Some(process) = self.sys.process(pid) {
                snap.rss_bytes = process.memory();
                snap.virtual_bytes = process.virtual_memory();
                snap.cpu_time_ms = process.accumulated_cpu_time() as f64;
                snap.thread_count = process.tasks().map(|t| t.len().max(1));
                let disk = process.disk_usage();
                snap.disk_read_bytes = disk.total_read_bytes;
                snap.disk_written_bytes = disk.total_written_bytes;
            }
        }

        os_extras(&mut snap);
        snap
    }
}

#[cfg(unix)]
fn os_extras(snap: &mut HardwareSnapshot) {
    // SAFETY: getrusage only writes into the zero-initialised struct we pass it.
    let usage = unsafe {
        let mut usage: libc::rusage = std::mem::zeroed();
        (libc::getrusage(libc::RUSAGE_SELF, &mut usage) == 0).then_some(usage)
    };
    if let Some(u) = usage {
        let ms = |tv: libc::timeval| tv.tv_sec as f64 * 1000.0 + tv.tv_usec as f64 / 1000.0;
        snap.user_cpu_ms = Some(ms(u.ru_utime));
        snap.sys_cpu_ms = Some(ms(u.ru_stime));
        snap.minor_faults = Some(u.ru_minflt as u64);
        snap.major_faults = Some(u.ru_majflt as u64);
        // ru_maxrss is bytes on macOS and kilobytes elsewhere.
        let max_rss = u.ru_maxrss as u64;
        snap.os_peak_rss_bytes = Some(if cfg!(target_os = "macos") { max_rss } else { max_rss * 1024 });
    }

    #[cfg(target_os = "linux")]
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        let kb = |line: &str| line.split_whitespace().nth(1).and_then(|v| v.parse::<u64>().ok()).map(|k| k * 1024);
        for line in status.lines() {
            match line.split(':').next() {
                Some("RssAnon") => snap.rss_anon_bytes = kb(line),
                Some("RssFile") => snap.rss_file_bytes = kb(line),
                Some("VmHWM") => snap.os_peak_rss_bytes = kb(line).or(snap.os_peak_rss_bytes),
                _ => {}
            }
        }
    }
}

#[cfg(not(unix))]
fn os_extras(_snap: &mut HardwareSnapshot) {}

/// Recorded stage event in the execution timeline
#[derive(Debug, Clone)]
pub struct TimelineStageEvent {
    pub stage_name: String,
    pub wall_duration: Duration,
    pub cumulative_offset: Duration,
    // RAM
    pub rss_start_mb: f64,
    pub rss_end_mb: f64,
    pub rss_delta_mb: f64,
    pub peak_rss_mb: f64,
    pub heap_anon_mb: Option<f64>,
    pub mmap_file_mb: Option<f64>,
    pub ram_budget_pct: f64,
    // CPU
    pub cpu_time_ms: f64,
    pub user_cpu_ms: Option<f64>,
    pub sys_cpu_ms: Option<f64>,
    pub cpu_load_pct: f64,
    pub active_threads: Option<usize>,
    // Disk
    pub disk_read_bytes: u64,
    pub disk_read_mb_per_sec: f64,
    pub minor_faults: Option<u64>,
    pub major_faults: Option<u64>,
    // Extra stage description
    pub details: String,
    // Optional GPU/Frame data
    pub fps_throughput: Option<f64>,
    pub frame_time_us: Option<f64>,
    pub visible_nodes: Option<usize>,
    pub visible_wires: Option<usize>,
}

/// High-resolution timeline recorder tracking process resources across a run.
pub struct TimelineTracker {
    start_time: Instant,
    probe: ProcessProbe,
    current_snapshot: HardwareSnapshot,
    observed_peak_rss: u64,
    pub budget: TelemetryBudget,
    pub gpu: Option<GpuDeviceInfo>,
    pub events: Vec<TimelineStageEvent>,
}

impl TimelineTracker {
    pub fn new(budget: TelemetryBudget) -> Self {
        let mut probe = ProcessProbe::new();
        let snapshot = probe.capture();
        Self {
            start_time: snapshot.timestamp,
            observed_peak_rss: snapshot.rss_bytes,
            probe,
            current_snapshot: snapshot,
            budget,
            gpu: GpuDeviceInfo::detect_headless(),
            events: Vec::with_capacity(32),
        }
    }

    pub fn gpu_summary(&self) -> String {
        self.gpu.as_ref().map_or_else(|| "no GPU adapter found".to_string(), GpuDeviceInfo::summary)
    }

    /// Records completion of a stage and appends a hardware telemetry event
    pub fn record_stage(
        &mut self,
        stage_name: impl Into<String>,
        details: impl Into<String>,
        fps_throughput: Option<f64>,
        frame_time_us: Option<f64>,
        visible_nodes: Option<usize>,
        visible_wires: Option<usize>,
    ) {
        let t1 = self.probe.capture();
        let t0 = self.current_snapshot;
        self.current_snapshot = t1;
        self.observed_peak_rss = self.observed_peak_rss.max(t1.rss_bytes);

        let mb = |b: u64| b as f64 / 1_048_576.0;
        let delta = |a: Option<f64>, b: Option<f64>| Some((b? - a?).max(0.0));
        let wall_duration = t1.timestamp.duration_since(t0.timestamp);
        let wall_ms = wall_duration.as_secs_f64() * 1000.0;
        let cpu_time_ms = (t1.cpu_time_ms - t0.cpu_time_ms).max(0.0);
        let wall_secs = wall_duration.as_secs_f64();
        let disk_read_bytes = t1.disk_read_bytes.saturating_sub(t0.disk_read_bytes);

        let event = TimelineStageEvent {
            stage_name: stage_name.into(),
            wall_duration,
            cumulative_offset: t1.timestamp.duration_since(self.start_time),
            rss_start_mb: mb(t0.rss_bytes),
            rss_end_mb: mb(t1.rss_bytes),
            rss_delta_mb: (t1.rss_bytes as f64 - t0.rss_bytes as f64) / 1_048_576.0,
            peak_rss_mb: mb(t1.os_peak_rss_bytes.unwrap_or(0).max(self.observed_peak_rss)),
            heap_anon_mb: t1.rss_anon_bytes.map(mb),
            mmap_file_mb: t1.rss_file_bytes.map(mb),
            ram_budget_pct: t1.rss_bytes as f64 / self.budget.ram_bytes.max(1) as f64 * 100.0,
            cpu_time_ms,
            user_cpu_ms: delta(t0.user_cpu_ms, t1.user_cpu_ms),
            sys_cpu_ms: delta(t0.sys_cpu_ms, t1.sys_cpu_ms),
            cpu_load_pct: if wall_ms > 0.001 { cpu_time_ms / wall_ms * 100.0 } else { 0.0 },
            active_threads: t1.thread_count,
            disk_read_bytes,
            disk_read_mb_per_sec: if wall_secs > 0.0001 { mb(disk_read_bytes) / wall_secs } else { 0.0 },
            minor_faults: t1.minor_faults.zip(t0.minor_faults).map(|(a, b)| a.saturating_sub(b)),
            major_faults: t1.major_faults.zip(t0.major_faults).map(|(a, b)| a.saturating_sub(b)),
            details: details.into(),
            fps_throughput,
            frame_time_us,
            visible_nodes,
            visible_wires,
        };

        self.print_event(&event);
        self.events.push(event);
    }

    fn print_event(&self, e: &TimelineStageEvent) {
        let opt_mb = |v: Option<f64>| v.map_or("n/a".to_string(), |v| format!("{v:.2} MB"));
        println!(
            "[Timeline +{:>6.2}s] ─── Stage: {} ({:>8.2?})",
            e.cumulative_offset.as_secs_f64(),
            e.stage_name,
            e.wall_duration
        );
        println!(
            "  • RAM:  {:>7.2} MB (Δ {:>+6.2} MB) | Heap: {} | Peak: {:>7.2} MB | [{:>4.1}% of {:.2} GB]",
            e.rss_end_mb,
            e.rss_delta_mb,
            opt_mb(e.heap_anon_mb),
            e.peak_rss_mb,
            e.ram_budget_pct,
            self.budget.ram_gb()
        );
        let split = match (e.user_cpu_ms, e.sys_cpu_ms) {
            (Some(u), Some(s)) => format!("User: {u:>6.1} ms, Sys: {s:>5.1} ms"),
            _ => format!("CPU time: {:.1} ms", e.cpu_time_ms),
        };
        let threads = e.active_threads.map_or("n/a".to_string(), |t| t.to_string());
        println!("  • CPU:  Load {:>5.1}% ({split}) | Threads: {threads}", e.cpu_load_pct);
        let faults = match (e.minor_faults, e.major_faults) {
            (Some(minor), Some(major)) => format!(" | Faults: {minor:>5} minor, {major:>2} major"),
            _ => String::new(),
        };
        println!(
            "  • DISK: Read {:>6.2} MB ({:>6.1} MB/s){faults}",
            e.disk_read_bytes as f64 / 1_048_576.0,
            e.disk_read_mb_per_sec
        );

        if let (Some(fps), Some(frame_us)) = (e.fps_throughput, e.frame_time_us) {
            let headroom = self.budget.frame_budget_us() / frame_us;
            println!(
                "  • GPU:  Frame {:>7.2} µs ({:>9.1} FPS) | Headroom: {:>5.1}x vs {:.0} FPS | Visible: {} nodes, {} wires",
                frame_us,
                fps,
                headroom,
                self.budget.target_fps,
                e.visible_nodes.unwrap_or(0),
                e.visible_wires.unwrap_or(0)
            );
        }
        if !e.details.is_empty() {
            println!("  • Info: {}", e.details);
        }
        println!();
    }

    /// Prints a comprehensive summary table covering the entire run
    pub fn print_timeline_summary(&self) {
        println!("====================================================================================================");
        println!("                         FULL EXECUTION TIMELINE & HARDWARE AUDIT REPORT                           ");
        println!("====================================================================================================");
        println!("  GPU:             {}", self.gpu_summary());
        println!("  Budget:          {}", self.budget.describe());
        println!("----------------------------------------------------------------------------------------------------");
        println!(
            "{:<4} | {:<28} | {:<9} | {:<10} | {:<7} | {:<8} | {:<12}",
            "Step", "Pipeline Stage", "Duration", "End RAM", "Peak RAM", "CPU Load", "Status"
        );
        println!("----------------------------------------------------------------------------------------------------");

        let fps_pass = format!("{:.0} FPS PASS", self.budget.target_fps);
        for (idx, e) in self.events.iter().enumerate() {
            let status = if e.ram_budget_pct > 100.0 {
                "RAM OVERFLOW"
            } else if let Some(frame_us) = e.frame_time_us {
                if frame_us <= self.budget.frame_budget_us() {
                    fps_pass.as_str()
                } else {
                    "FPS DROPPED"
                }
            } else {
                "PASS"
            };

            println!(
                "#{:<3} | {:<28} | {:>9.2?} | {:>7.2} MB | {:>7.2} MB | {:>6.1}% | {:<12}",
                idx + 1,
                truncate_chars(&e.stage_name, 28),
                e.wall_duration,
                e.rss_end_mb,
                e.peak_rss_mb,
                e.cpu_load_pct,
                status
            );
        }
        println!("====================================================================================================\n");
    }
}

/// First `max` characters (not bytes), so non-ASCII stage names cannot split a code point.
fn truncate_chars(s: &str, max: usize) -> &str {
    s.char_indices().nth(max).map_or(s, |(i, _)| &s[..i])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_is_char_safe() {
        assert_eq!(truncate_chars("Rendu des nœuds éléments à l'écran", 12), "Rendu des nœ");
        assert_eq!(truncate_chars("short", 28), "short");
    }

    #[test]
    fn snapshot_reports_this_process() {
        let mut probe = ProcessProbe::new();
        let a = probe.capture();
        let buf = vec![1u8; 32 * 1024 * 1024];
        std::hint::black_box(&buf);
        let b = probe.capture();
        assert!(a.rss_bytes > 0 && b.rss_bytes > 0);
        assert!(b.cpu_time_ms >= a.cpu_time_ms);
    }

    #[test]
    fn budget_math() {
        let b = TelemetryBudget { ram_bytes: 8 * 1_073_741_824, target_fps: 120.0 };
        assert!((b.frame_budget_us() - 8333.33).abs() < 0.01);
        assert_eq!(b.ram_gb(), 8.0);
        assert!(TelemetryBudget::for_this_machine().ram_bytes > 0);
    }
}
