//! What the machine does while the game runs, sampled once a second:
//! CPU, memory and disk of the game and its children, the whole system, and
//! (Windows) the GPU the game uses.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use oedvi_common::stats::Specs;
use sysinfo::{Pid, ProcessesToUpdate, System};
use tokio_util::sync::CancellationToken;

use crate::metrics::Recorder;

const MB: f64 = 1024.0 * 1024.0;

/// Pids of `root` and of everything it started.
fn tree(root: u32, parents: &HashMap<u32, u32>) -> HashSet<u32> {
    let mut set = HashSet::from([root]);
    loop {
        let before = set.len();
        for (pid, parent) in parents {
            if set.contains(parent) {
                set.insert(*pid);
            }
        }
        if set.len() == before {
            return set;
        }
    }
}

/// Samples the game's process tree and the system.
pub struct Sampler {
    sys: System,
    root: u32,
    last: Instant,
}

impl Sampler {
    pub fn new(root: u32) -> Option<Self> {
        let mut sys = System::new().ok()?;
        sys.refresh_cpu_usage();
        sys.refresh_processes(ProcessesToUpdate::All, true);
        Some(Self { sys, root, last: Instant::now() })
    }

    pub fn sample(&mut self) -> BTreeMap<&'static str, f64> {
        let dt = self.last.elapsed().as_secs_f64().max(0.1);
        self.last = Instant::now();
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();
        self.sys.refresh_processes(ProcessesToUpdate::All, true);
        let parents: HashMap<u32, u32> =
            self.sys.processes().iter().filter_map(|(p, pr)| Some((p.as_u32(), pr.parent()?.as_u32()))).collect();
        let mine = tree(self.root, &parents);
        let cores = self.sys.cpus().len().max(1) as f64;
        let (mut cpu, mut ram, mut read, mut written) = (0.0, 0.0, 0.0, 0.0);
        for pid in &mine {
            let Some(p) = self.sys.process(Pid::from_u32(*pid)) else { continue };
            // 100 = one core: as a share of the whole machine, like the Task Manager.
            cpu += p.cpu_usage() as f64 / cores;
            ram += p.memory() as f64;
            let d = p.disk_usage();
            read += d.read_bytes as f64;
            written += d.written_bytes as f64;
        }
        BTreeMap::from([
            ("proc_cpu", cpu.min(100.0)),
            ("proc_ram", ram / MB),
            ("disk_read", read / MB / dt),
            ("disk_write", written / MB / dt),
            ("sys_cpu", self.sys.global_cpu_usage() as f64),
            ("sys_ram", self.sys.used_memory() as f64 / MB),
        ])
    }
}

/// CPU, memory, system and graphics cards of this PC.
pub async fn specs() -> Specs {
    let base = tokio::task::spawn_blocking(|| {
        let mut sys = System::new().ok()?;
        sys.refresh_cpu_all();
        sys.refresh_memory();
        let os = System::long_os_version().or_else(|_| System::name()).unwrap_or_default();
        let kernel = System::kernel_version().unwrap_or_default();
        Some(Specs {
            cpu: sys.cpus().first().map(|c| c.brand().trim().to_string()).unwrap_or_default(),
            cores: System::physical_core_count().unwrap_or(0) as u32,
            threads: sys.cpus().len() as u32,
            ram_mb: (sys.total_memory() as f64 / MB) as u64,
            os: if kernel.is_empty() { os } else { format!("{os} ({kernel})") },
            gpus: Vec::new(),
        })
    })
    .await
    .ok()
    .flatten()
    .unwrap_or_default();
    Specs { gpus: gpu::cards().await, ..base }
}

/// Samples until `stop` fires: records into `recorder` every second.
pub fn spawn(pid: u32, recorder: Arc<Recorder>, stop: CancellationToken) {
    let rec = recorder.clone();
    let token = stop.clone();
    tokio::spawn(async move {
        // `sysinfo` blocks briefly: keep it off the async threads.
        let mut sampler = Sampler::new(pid);
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                _ = token.cancelled() => return,
                _ = tick.tick() => {}
            }
            let Some(mut s) = sampler.take() else { return };
            let rec = rec.clone();
            let Ok((s, values)) = tokio::task::spawn_blocking(move || {
                let v = s.sample();
                (s, v)
            })
            .await
            else {
                return;
            };
            sampler = Some(s);
            for (name, v) in values {
                rec.push(name, v);
            }
        }
    });
    #[cfg(windows)]
    tokio::spawn(gpu::run(pid, recorder, stop));
    #[cfg(not(windows))]
    let _ = (recorder, stop);
}

/// GPU use of one process, from the Windows performance counters
/// (`typeperf`, which ships with Windows and works for any GPU vendor).
pub mod gpu {
    use std::collections::BTreeMap;

    use oedvi_common::stats::Gpu;

    /// Graphics cards with their driver version, from `Win32_VideoController`.
    pub async fn cards() -> Vec<Gpu> {
        #[cfg(windows)]
        {
            use tokio::process::Command;
            let script = "Get-CimInstance Win32_VideoController | Select-Object Name,DriverVersion,\
                @{n='Date';e={if($_.DriverDate){$_.DriverDate.ToString('yyyy-MM-dd')}else{''}}} | ConvertTo-Json -Compress";
            let out = Command::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-Command", script])
                .creation_flags(0x0800_0000)
                .stdin(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .output();
            match tokio::time::timeout(std::time::Duration::from_secs(15), out).await {
                Ok(Ok(o)) => parse_cards(&String::from_utf8_lossy(&o.stdout)),
                _ => Vec::new(),
            }
        }
        #[cfg(not(windows))]
        Vec::new()
    }

    /// PowerShell prints one object, or an array when there are several.
    pub fn parse_cards(json: &str) -> Vec<Gpu> {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(json.trim()) else { return Vec::new() };
        let items = match v {
            serde_json::Value::Array(a) => a,
            o @ serde_json::Value::Object(_) => vec![o],
            _ => return Vec::new(),
        };
        let text = |o: &serde_json::Value, k: &str| o.get(k).and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        items
            .iter()
            .map(|o| Gpu { name: text(o, "Name"), driver: text(o, "DriverVersion"), driver_date: text(o, "Date") })
            .filter(|g| !g.name.is_empty())
            .collect()
    }

    /// Which column of a `typeperf` row is what.
    #[derive(Debug, PartialEq)]
    pub struct Columns {
        /// Utilization of an engine: `(engine type, column)`.
        engines: Vec<(String, usize)>,
        /// Dedicated video memory: columns.
        memory: Vec<usize>,
    }

    fn cells(line: &str) -> Vec<&str> {
        line.trim().trim_matches('"').split("\",\"").collect()
    }

    impl Columns {
        pub fn from_header(line: &str) -> Option<Self> {
            let mut c = Columns { engines: Vec::new(), memory: Vec::new() };
            for (i, name) in cells(line).into_iter().enumerate().skip(1) {
                if name.contains("Utilization Percentage") {
                    let kind = name.split("engtype_").nth(1).and_then(|s| s.split(')').next()).unwrap_or("?");
                    c.engines.push((kind.to_string(), i));
                } else if name.contains("Dedicated Usage") {
                    c.memory.push(i);
                }
            }
            (!c.engines.is_empty() || !c.memory.is_empty()).then_some(c)
        }

        /// `(gpu %, video memory MB)` of a data row. The busiest engine type
        /// counts, as in the Task Manager (3D, compute, copy…).
        pub fn reduce(&self, line: &str) -> Option<(f64, f64)> {
            let row = cells(line);
            let at = |i: usize| row.get(i)?.trim().parse::<f64>().ok();
            let mut per_kind: BTreeMap<&str, f64> = BTreeMap::new();
            for (kind, i) in &self.engines {
                *per_kind.entry(kind).or_default() += at(*i)?;
            }
            let gpu = per_kind.values().fold(0.0_f64, |a, b| a.max(*b)).min(100.0);
            let mem: f64 = self.memory.iter().filter_map(|i| at(*i)).sum();
            Some((gpu, mem / super::MB))
        }
    }

    #[cfg(windows)]
    pub async fn run(
        pid: u32,
        recorder: std::sync::Arc<crate::metrics::Recorder>,
        stop: tokio_util::sync::CancellationToken,
    ) {
        use std::time::Duration;
        use tokio::io::{AsyncBufReadExt, BufReader};
        use tokio::process::Command;
        loop {
            let mut child = match Command::new("typeperf")
                .args([
                    &format!("\\GPU Engine(pid_{pid}*)\\Utilization Percentage"),
                    &format!("\\GPU Process Memory(pid_{pid}*)\\Dedicated Usage"),
                    "-si",
                    "1",
                ])
                .creation_flags(0x0800_0000)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true)
                .spawn()
            {
                Ok(c) => c,
                Err(_) => return,
            };
            let mut lines = BufReader::new(child.stdout.take().expect("piped")).lines();
            let mut columns = None;
            // Counters appear once the game starts drawing, and a new
            // instance is only seen by a new `typeperf`: restart now and then.
            let deadline = tokio::time::sleep(Duration::from_secs(20));
            tokio::pin!(deadline);
            loop {
                tokio::select! {
                    _ = stop.cancelled() => return,
                    _ = &mut deadline => break,
                    line = lines.next_line() => match line {
                        Ok(Some(line)) => {
                            if columns.is_none() {
                                columns = Columns::from_header(&line);
                            } else if let Some((gpu, mem)) = columns.as_ref().and_then(|c| c.reduce(&line)) {
                                recorder.push("proc_gpu", gpu);
                                recorder.push("proc_vram", mem);
                            }
                        }
                        _ => break,
                    },
                }
            }
            let _ = child.kill().await;
            tokio::select! {
                _ = stop.cancelled() => return,
                _ = tokio::time::sleep(Duration::from_secs(if columns.is_some() { 0 } else { 3 })) => {}
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        const HEADER: &str = r#""(PDH-CSV 4.0)","\\PC\GPU Engine(pid_42_luid_0x0_0x1_phys_0_eng_0_engtype_3D)\Utilization Percentage","\\PC\GPU Engine(pid_42_luid_0x0_0x1_phys_0_eng_1_engtype_3D)\Utilization Percentage","\\PC\GPU Engine(pid_42_luid_0x0_0x1_phys_0_eng_2_engtype_Copy)\Utilization Percentage","\\PC\GPU Process Memory(pid_42_luid_0x0_0x1_phys_0)\Dedicated Usage""#;

        #[test]
        fn reads_the_cards_powershell_lists() {
            let one = r#"{"Name":"AMD Radeon RX 7800 XT","DriverVersion":"32.0.21013.1000","Date":"2025-11-04"}"#;
            let g = parse_cards(one);
            assert_eq!(
                (g[0].name.as_str(), g[0].driver.as_str(), g[0].driver_date.as_str()),
                ("AMD Radeon RX 7800 XT", "32.0.21013.1000", "2025-11-04")
            );
            let two = format!(
                "[{one},{{\"Name\":\"\",\"DriverVersion\":\"1\"}},{{\"Name\":\"Basic\",\"DriverVersion\":null}}]"
            );
            assert_eq!(parse_cards(&two).len(), 2);
            assert!(parse_cards("garbage").is_empty());
        }

        #[test]
        fn reads_typeperf_rows() {
            let c = Columns::from_header(HEADER).unwrap();
            let (gpu, mem) = c.reduce(r#""11/25/2026 19:10:00.123","30.5","20","70","1073741824""#).unwrap();
            // 3D engines add up (50.5), copy is 70: the busiest type wins.
            assert_eq!(gpu, 70.0);
            assert_eq!(mem, 1024.0);
            let (gpu, _) = c.reduce(r#""t","60","50","1","0""#).unwrap();
            assert_eq!(gpu, 100.0, "capped");
            assert!(c.reduce(r#""t"," ","1","1","1""#).is_none());
            assert!(Columns::from_header("Error: No valid counters.").is_none());
        }
    }
}
