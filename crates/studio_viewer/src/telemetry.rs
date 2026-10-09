use std::time::{Duration, Instant};

/// Maximum RAM budget specified by user: 12.0 GB
pub const RAM_BUDGET_BYTES: u64 = 12 * 1024 * 1024 * 1024;
/// Locked frame budget for 165 FPS: 6.0606 ms (6060.6 µs)
pub const TARGET_165_FPS_US: f64 = 1_000_000.0 / 165.0; // ~6060.6 µs

/// Hardware GPU metrics detected from system
#[derive(Debug, Clone)]
pub struct GpuDeviceInfo {
    pub model: String,
    pub pci_slot: String,
    pub driver_version: String,
}

impl GpuDeviceInfo {
    pub fn detect() -> Self {
        let mut model = "Unknown GPU".to_string();
        let mut pci_slot = "PCIe".to_string();
        let mut driver_version = "Generic DRM".to_string();

        // 1. Try reading NVIDIA driver info from /proc/driver/nvidia
        let pci_dirs = ["0000:01:00.0", "0000:00:02.0", "0000:02:00.0"];
        for slot in pci_dirs {
            let path = format!("/proc/driver/nvidia/gpus/{}/information", slot);
            if let Ok(content) = std::fs::read_to_string(&path) {
                pci_slot = slot.to_string();
                for line in content.lines() {
                    if let Some(rest) = line.strip_prefix("Model:") {
                        model = rest.trim().to_string();
                    } else if let Some(rest) = line.strip_prefix("GPU Firmware:") {
                        driver_version = format!("NVIDIA Driver {}", rest.trim());
                    }
                }
                return Self { model, pci_slot, driver_version };
            }
        }

        // 2. Fallback to /sys/class/drm/card*/device/uevent
        if let Ok(entries) = std::fs::read_dir("/sys/class/drm") {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("card") && !name.contains('-') {
                    let uevent_path = entry.path().join("device/uevent");
                    if let Ok(uevent) = std::fs::read_to_string(uevent_path) {
                        if uevent.contains("DRIVER=nvidia") {
                            model = "NVIDIA GeForce RTX (Discrete GPU)".to_string();
                            driver_version = "nvidia.ko".to_string();
                            return Self { model, pci_slot: "0000:01:00.0".to_string(), driver_version };
                        } else if uevent.contains("DRIVER=i915") {
                            model = "Intel Iris Xe / UHD Graphics".to_string();
                            driver_version = "i915.ko".to_string();
                        }
                    }
                }
            }
        }

        Self { model, pci_slot, driver_version }
    }
}

/// Instantaneous hardware snapshot capturing RAM, CPU, Disk and Process state
#[derive(Debug, Clone, Copy)]
pub struct HardwareSnapshot {
    pub timestamp: Instant,
    // RAM Metrics (bytes)
    pub rss_bytes: u64,
    pub rss_anon_bytes: u64,
    pub rss_file_bytes: u64,
    pub vm_size_bytes: u64,
    pub vm_hwm_bytes: u64, // High-water mark (Peak physical RAM)
    pub vm_peak_bytes: u64, // Peak virtual memory
    // CPU Metrics
    pub user_cpu_ms: f64,
    pub sys_cpu_ms: f64,
    pub thread_count: usize,
    // Disk I/O Metrics
    pub io_read_bytes: u64,
    pub io_write_bytes: u64,
    pub io_rchar_bytes: u64,
    pub io_syscr: u64,
    pub io_syscw: u64,
    // Page faults
    pub minor_faults: u64,
    pub major_faults: u64,
}

impl HardwareSnapshot {
    pub fn capture() -> Self {
        let timestamp = Instant::now();
        let page_size = 4096u64;

        // 1. Read /proc/self/statm for fast RSS
        let mut rss_bytes = 0u64;
        let mut vm_size_bytes = 0u64;
        if let Ok(statm) = std::fs::read_to_string("/proc/self/statm") {
            let mut parts = statm.split_whitespace();
            if let Some(vms) = parts.next().and_then(|s| s.parse::<u64>().ok()) {
                vm_size_bytes = vms * page_size;
            }
            if let Some(rss) = parts.next().and_then(|s| s.parse::<u64>().ok()) {
                rss_bytes = rss * page_size;
            }
        }

        // 2. Read /proc/self/status for detailed heap / peak metrics
        let mut rss_anon_bytes = 0u64;
        let mut rss_file_bytes = 0u64;
        let mut vm_hwm_bytes = rss_bytes;
        let mut vm_peak_bytes = vm_size_bytes;
        let mut thread_count = 1usize;

        if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
            for line in status.lines() {
                if let Some(rest) = line.strip_prefix("RssAnon:") {
                    if let Some(kb) = parse_kb(rest) { rss_anon_bytes = kb * 1024; }
                } else if let Some(rest) = line.strip_prefix("RssFile:") {
                    if let Some(kb) = parse_kb(rest) { rss_file_bytes = kb * 1024; }
                } else if let Some(rest) = line.strip_prefix("VmHWM:") {
                    if let Some(kb) = parse_kb(rest) { vm_hwm_bytes = kb * 1024; }
                } else if let Some(rest) = line.strip_prefix("VmPeak:") {
                    if let Some(kb) = parse_kb(rest) { vm_peak_bytes = kb * 1024; }
                } else if let Some(rest) = line.strip_prefix("Threads:") {
                    if let Ok(tc) = rest.trim().parse::<usize>() { thread_count = tc; }
                }
            }
        }

        // 3. Read /proc/self/stat for CPU ticks
        let mut user_cpu_ms = 0.0;
        let mut sys_cpu_ms = 0.0;
        if let Ok(stat) = std::fs::read_to_string("/proc/self/stat") {
            // Find closing paren of comm field to avoid spaces in process name
            if let Some(idx) = stat.rfind(')') {
                let rest = &stat[idx + 1..];
                let fields: Vec<&str> = rest.split_whitespace().collect();
                // Field index in rest (0 is state):
                // 11 = utime (ticks), 12 = stime (ticks)
                if fields.len() > 12 {
                    if let (Ok(uticks), Ok(sticks)) = (fields[11].parse::<u64>(), fields[12].parse::<u64>()) {
                        let clk_tck = 100.0f64; // Standard Linux USER_HZ
                        user_cpu_ms = (uticks as f64 / clk_tck) * 1000.0;
                        sys_cpu_ms = (sticks as f64 / clk_tck) * 1000.0;
                    }
                }
            }
        }

        // 4. Read /proc/self/io for disk metrics
        let mut io_read_bytes = 0u64;
        let mut io_write_bytes = 0u64;
        let mut io_rchar_bytes = 0u64;
        let mut io_syscr = 0u64;
        let mut io_syscw = 0u64;

        if let Ok(io) = std::fs::read_to_string("/proc/self/io") {
            for line in io.lines() {
                if let Some(rest) = line.strip_prefix("read_bytes:") {
                    if let Ok(b) = rest.trim().parse::<u64>() { io_read_bytes = b; }
                } else if let Some(rest) = line.strip_prefix("write_bytes:") {
                    if let Ok(b) = rest.trim().parse::<u64>() { io_write_bytes = b; }
                } else if let Some(rest) = line.strip_prefix("rchar:") {
                    if let Ok(b) = rest.trim().parse::<u64>() { io_rchar_bytes = b; }
                } else if let Some(rest) = line.strip_prefix("syscr:") {
                    if let Ok(b) = rest.trim().parse::<u64>() { io_syscr = b; }
                } else if let Some(rest) = line.strip_prefix("syscw:") {
                    if let Ok(b) = rest.trim().parse::<u64>() { io_syscw = b; }
                }
            }
        }

        // 5. Read page faults via libc::getrusage
        let mut minor_faults = 0u64;
        let mut major_faults = 0u64;
        unsafe {
            let mut usage: libc::rusage = std::mem::zeroed();
            if libc::getrusage(libc::RUSAGE_SELF, &mut usage) == 0 {
                minor_faults = usage.ru_minflt as u64;
                major_faults = usage.ru_majflt as u64;
            }
        }

        Self {
            timestamp,
            rss_bytes,
            rss_anon_bytes,
            rss_file_bytes,
            vm_size_bytes,
            vm_hwm_bytes,
            vm_peak_bytes,
            user_cpu_ms,
            sys_cpu_ms,
            thread_count,
            io_read_bytes,
            io_write_bytes,
            io_rchar_bytes,
            io_syscr,
            io_syscw,
            minor_faults,
            major_faults,
        }
    }
}

fn parse_kb(s: &str) -> Option<u64> {
    s.trim().split_whitespace().next()?.parse::<u64>().ok()
}

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
    pub heap_anon_mb: f64,
    pub mmap_file_mb: f64,
    pub ram_budget_pct: f64,
    // CPU
    pub user_cpu_ms: f64,
    pub sys_cpu_ms: f64,
    pub cpu_load_pct: f64,
    pub active_threads: usize,
    // Disk
    pub disk_read_bytes: u64,
    pub disk_read_mb_per_sec: f64,
    pub disk_syscalls: u64,
    pub minor_faults: u64,
    pub major_faults: u64,
    // Extra stage description
    pub details: String,
    // Optional GPU/Frame data
    pub fps_throughput: Option<f64>,
    pub frame_time_us: Option<f64>,
    pub visible_nodes: Option<usize>,
    pub visible_wires: Option<usize>,
}

/// High-resolution Timeline Recorder tracking hardware across execution
pub struct TimelineTracker {
    start_time: Instant,
    current_snapshot: HardwareSnapshot,
    pub gpu: GpuDeviceInfo,
    pub events: Vec<TimelineStageEvent>,
}

impl TimelineTracker {
    pub fn new() -> Self {
        let snapshot = HardwareSnapshot::capture();
        Self {
            start_time: snapshot.timestamp,
            current_snapshot: snapshot,
            gpu: GpuDeviceInfo::detect(),
            events: Vec::with_capacity(32),
        }
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
        let t1 = HardwareSnapshot::capture();
        let t0 = self.current_snapshot;
        self.current_snapshot = t1;

        let wall_duration = t1.timestamp.duration_since(t0.timestamp);
        let cumulative_offset = t1.timestamp.duration_since(self.start_time);

        let wall_ms = wall_duration.as_secs_f64() * 1000.0;
        let user_cpu_ms = (t1.user_cpu_ms - t0.user_cpu_ms).max(0.0);
        let sys_cpu_ms = (t1.sys_cpu_ms - t0.sys_cpu_ms).max(0.0);
        let total_cpu_ms = user_cpu_ms + sys_cpu_ms;
        let cpu_load_pct = if wall_ms > 0.001 {
            (total_cpu_ms / wall_ms) * 100.0
        } else {
            0.0
        };

        let rss_start_mb = t0.rss_bytes as f64 / 1_048_576.0;
        let rss_end_mb = t1.rss_bytes as f64 / 1_048_576.0;
        let rss_delta_mb = (t1.rss_bytes as i64 - t0.rss_bytes as i64) as f64 / 1_048_576.0;
        let peak_rss_mb = t1.vm_hwm_bytes as f64 / 1_048_576.0;
        let heap_anon_mb = t1.rss_anon_bytes as f64 / 1_048_576.0;
        let mmap_file_mb = t1.rss_file_bytes as f64 / 1_048_576.0;
        let ram_budget_pct = (t1.rss_bytes as f64 / RAM_BUDGET_BYTES as f64) * 100.0;

        let disk_read_bytes = t1.io_read_bytes.saturating_sub(t0.io_read_bytes);
        let wall_secs = wall_duration.as_secs_f64();
        let disk_read_mb_per_sec = if wall_secs > 0.0001 {
            (disk_read_bytes as f64 / 1_048_576.0) / wall_secs
        } else {
            0.0
        };
        let disk_syscalls = t1.io_syscr.saturating_sub(t0.io_syscr);
        let minor_faults = t1.minor_faults.saturating_sub(t0.minor_faults);
        let major_faults = t1.major_faults.saturating_sub(t0.major_faults);

        let event = TimelineStageEvent {
            stage_name: stage_name.into(),
            wall_duration,
            cumulative_offset,
            rss_start_mb,
            rss_end_mb,
            rss_delta_mb,
            peak_rss_mb,
            heap_anon_mb,
            mmap_file_mb,
            ram_budget_pct,
            user_cpu_ms,
            sys_cpu_ms,
            cpu_load_pct,
            active_threads: t1.thread_count,
            disk_read_bytes,
            disk_read_mb_per_sec,
            disk_syscalls,
            minor_faults,
            major_faults,
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
        println!(
            "[Timeline +{:>6.2}s] ─── Stage: {} ({:>8.2?})",
            e.cumulative_offset.as_secs_f64(),
            e.stage_name,
            e.wall_duration
        );
        println!(
            "  • RAM:  {:>7.2} MB (Δ {:>+6.2} MB) | Heap: {:>7.2} MB | Peak: {:>7.2} MB | [{:>4.1}% of 12 GB]",
            e.rss_end_mb, e.rss_delta_mb, e.heap_anon_mb, e.peak_rss_mb, e.ram_budget_pct
        );
        println!(
            "  • CPU:  Load {:>5.1}% (User: {:>6.1} ms, Sys: {:>5.1} ms) | Threads: {:>2} active",
            e.cpu_load_pct, e.user_cpu_ms, e.sys_cpu_ms, e.active_threads
        );
        println!(
            "  • DISK: Read {:>6.2} MB ({:>6.1} MB/s) | Syscalls: {:>6} | Faults: {:>5} minor, {:>2} major",
            e.disk_read_bytes as f64 / 1_048_576.0,
            e.disk_read_mb_per_sec,
            e.disk_syscalls,
            e.minor_faults,
            e.major_faults
        );

        if let (Some(fps), Some(frame_us)) = (e.fps_throughput, e.frame_time_us) {
            let headroom = TARGET_165_FPS_US / frame_us;
            println!(
                "  • GPU:  Frame {:>7.2} µs ({:>9.1} FPS) | Headroom: {:>5.1}x vs 165 FPS | Visible: {} nodes, {} wires",
                frame_us, fps, headroom, e.visible_nodes.unwrap_or(0), e.visible_wires.unwrap_or(0)
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
        println!("  GPU Detected:    {} [{}] ({})", self.gpu.model, self.gpu.pci_slot, self.gpu.driver_version);
        println!("  Hardware Budget: Max RAM ≤ 12.00 GB | Locked 165 FPS Target (≤ 6.06 ms / 6060 µs per frame)");
        println!("----------------------------------------------------------------------------------------------------");
        println!(
            "{:<4} | {:<28} | {:<9} | {:<10} | {:<7} | {:<8} | {:<12}",
            "Step", "Pipeline Stage", "Duration", "End RAM", "Peak RAM", "CPU Load", "Status"
        );
        println!("----------------------------------------------------------------------------------------------------");

        for (idx, e) in self.events.iter().enumerate() {
            let status = if e.ram_budget_pct > 100.0 {
                "RAM OVERFLOW"
            } else if let Some(frame_us) = e.frame_time_us {
                if frame_us <= TARGET_165_FPS_US {
                    "165 FPS PASS"
                } else {
                    "FPS DROPPED"
                }
            } else {
                "PASS"
            };

            println!(
                "#{:<3} | {:<28} | {:>9.2?} | {:>7.2} MB | {:>7.2} MB | {:>6.1}% | {:<12}",
                idx + 1,
                if e.stage_name.len() > 28 { &e.stage_name[..28] } else { &e.stage_name },
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
