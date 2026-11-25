//! Reading what the game says about itself: the output is cleaned of
//! terminal colors and the FPS it reports are recorded for the session.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Instant;

use oedvi_common::stats::{Metric, Phase, Session};
use regex::Regex;

/// Two samples closer than this are one: a game printing every frame would
/// otherwise fill the memory.
const MIN_GAP_SECS: f64 = 0.5;

/// Removes terminal escape sequences (colors, cursor moves) from a line.
pub fn strip_ansi(line: &str) -> String {
    if !line.contains('\x1b') {
        return line.to_string();
    }
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            // CSI: parameters, then a final byte in @..~
            Some('[') => {
                for n in chars.by_ref() {
                    if ('@'..='~').contains(&n) {
                        break;
                    }
                }
            }
            // OSC: up to BEL or ESC \
            Some(']') => {
                while let Some(n) = chars.next() {
                    if n == '\x07' {
                        break;
                    }
                    if n == '\x1b' {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Numbers the game prints in its HUD line (`60 fps 16.6 ms/frame | gpu 40%
/// busy cpu 52% ram 11061 MB | 9 draws/frame`), by metric name.
const DEFAULT_METRICS: &[(&str, &[&str])] = &[
    // `N fps` first: in `1 fps 1735.7 ms/frame` the `fps N` form reads the ms.
    ("fps", &[r"(?i)(\d{1,4}(?:\.\d+)?)\s*fps\b", r"(?i)\bfps\b\s*[:=]?\s*(\d{1,4}(?:\.\d+)?)"]),
    // Only the HUD (`30.0 fps, 33.3 ms/frame` in the log, `1 fps 1735.7
    // ms/frame` in the title): other lines also say `ms/frame` for single
    // copies or passes, which are not the frame time.
    ("frame_ms", &[r"(?i)\bfps,?\s+(\d+(?:\.\d+)?)\s*ms/frame"]),
    ("gpu", &[r"(?i)\bgpu\s*[:=]?\s*(\d+(?:\.\d+)?)\s*%"]),
    ("cpu", &[r"(?i)\bcpu\s*[:=]?\s*(\d+(?:\.\d+)?)\s*%"]),
    ("ram", &[r"(?i)\bram\s*[:=]?\s*(\d+(?:\.\d+)?)\s*MB"]),
    ("draws", &[r"(?i)(\d+(?:\.\d+)?)\s*draws/frame"]),
];

/// Collects what one run of the game measures: the numbers in its output and
/// the ones the launcher samples from the system.
pub struct Recorder {
    patterns: Vec<(String, Vec<Regex>)>,
    phase_patterns: Vec<(String, Regex)>,
    started: Instant,
    started_at: u64,
    data: Mutex<Data>,
}

#[derive(Default)]
struct Data {
    series: BTreeMap<String, Vec<(f64, f64)>>,
    latest: BTreeMap<String, f64>,
    phases: Vec<Phase>,
}

fn compile(p: &str) -> Option<Regex> {
    Regex::new(p).ok().filter(|r| r.captures_len() > 1)
}

impl Recorder {
    /// `fps_patterns` and `custom` come from the builder; what does not
    /// compile is returned so it can be reported.
    pub fn new(
        fps_patterns: &[String],
        custom: &BTreeMap<String, String>,
        phases: &BTreeMap<String, String>,
    ) -> (Self, Vec<String>) {
        let mut bad = Vec::new();
        let mut table: BTreeMap<String, Vec<Regex>> = DEFAULT_METRICS
            .iter()
            .map(|(name, pats)| {
                (name.to_string(), pats.iter().map(|p| Regex::new(p).expect("static pattern")).collect())
            })
            .collect();
        let mut set = |name: &str, pats: Vec<&String>| {
            let ok: Vec<Regex> = pats
                .into_iter()
                .filter_map(|p| {
                    let r = compile(p);
                    if r.is_none() {
                        bad.push(p.clone());
                    }
                    r
                })
                .collect();
            if !ok.is_empty() {
                table.insert(name.to_string(), ok);
            }
        };
        if !fps_patterns.is_empty() {
            set("fps", fps_patterns.iter().collect());
        }
        for (name, p) in custom {
            set(name, vec![p]);
        }
        let started_at =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default();
        let phase_patterns = phases
            .iter()
            .filter_map(|(name, p)| match Regex::new(p) {
                Ok(r) => Some((name.clone(), r)),
                Err(_) => {
                    bad.push(p.clone());
                    None
                }
            })
            .collect();
        let this = Self {
            patterns: table.into_iter().collect(),
            phase_patterns,
            started: Instant::now(),
            started_at,
            data: Mutex::default(),
        };
        (this, bad)
    }

    /// Starts a phase now (from the output, or marked by hand). Nothing
    /// happens when it is already the current one.
    pub fn mark(&self, name: &str) {
        let t = self.started.elapsed().as_secs_f64();
        let mut d = self.data.lock().unwrap();
        if d.phases.last().is_none_or(|p| p.name != name) {
            d.phases.push(Phase { name: name.to_string(), start: t });
        }
    }

    /// The phase the game is in, if any started.
    pub fn phase(&self) -> Option<String> {
        self.data.lock().unwrap().phases.last().map(|p| p.name.clone())
    }

    /// Reads the numbers of a (cleaned) output line. `true` when it had any.
    pub fn feed(&self, line: &str) -> bool {
        if let Some((name, _)) = self.phase_patterns.iter().find(|(_, re)| re.is_match(line)) {
            self.mark(name);
        }
        let mut found = false;
        for (name, regexes) in &self.patterns {
            let value = regexes.iter().find_map(|re| {
                re.captures_iter(line)
                    .filter_map(|c| c.get(1)?.as_str().parse::<f64>().ok())
                    .find(|v| plausible(name, *v))
            });
            if let Some(v) = value {
                self.record(name, v, MIN_GAP_SECS);
                found = true;
            }
        }
        found
    }

    /// Records a measure of the system (no spacing: they come at 1 Hz).
    pub fn push(&self, name: &str, value: f64) {
        if value.is_finite() && value >= 0.0 {
            self.record(name, value, 0.0);
        }
    }

    fn record(&self, name: &str, value: f64, gap: f64) {
        let t = self.started.elapsed().as_secs_f64();
        let mut d = self.data.lock().unwrap();
        d.latest.insert(name.to_string(), value);
        let series = d.series.entry(name.to_string()).or_default();
        if series.last().is_none_or(|(last, _)| t - last >= gap) {
            series.push((t, value));
        }
    }

    /// The last value of every metric.
    pub fn latest(&self) -> BTreeMap<String, f64> {
        self.data.lock().unwrap().latest.clone()
    }

    pub fn elapsed(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    /// The finished session, or `None` when nothing was measured.
    pub fn finish(&self, id: String, builder: &str, commit: &str, machine: &str) -> Option<Session> {
        let phases = self.data.lock().unwrap().phases.clone();
        let metrics: BTreeMap<String, Metric> = self
            .data
            .lock()
            .unwrap()
            .series
            .iter()
            .filter_map(|(name, s)| Some((name.clone(), Metric::from_samples(s)?)))
            .collect();
        if metrics.is_empty() {
            return None;
        }
        let fps = metrics.get("fps");
        Some(Session {
            id,
            builder: builder.to_string(),
            commit: commit.chars().take(10).collect(),
            machine: machine.to_string(),
            os: std::env::consts::OS.to_string(),
            launcher: env!("CARGO_PKG_VERSION").to_string(),
            started_at: self.started_at,
            duration_secs: self.started.elapsed().as_secs_f64(),
            samples: fps.map_or(0, |m| m.samples),
            avg: fps.map_or(0.0, |m| m.avg),
            min: fps.map_or(0.0, |m| m.min),
            max: fps.map_or(0.0, |m| m.max),
            low1: fps.map_or(0.0, |m| m.p1),
            series: fps.map(|m| m.series.clone()).unwrap_or_default(),
            metrics,
            specs: None,
            phases,
        })
    }
}

fn plausible(name: &str, v: f64) -> bool {
    (0.0..1e9).contains(&v) && (name != "fps" || (v > 0.0 && v <= 1000.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_colors_like_the_journal_of_a_tracing_game() {
        let raw = "\x1b[2m2026-11-25T17:49:41Z\x1b[0m \x1b[32m INFO\x1b[0m \x1b[2mvictory_gpu::pipeline\x1b[0m\x1b[2m:\x1b[0m first bind";
        assert_eq!(strip_ansi(raw), "2026-11-25T17:49:41Z  INFO victory_gpu::pipeline: first bind");
        assert_eq!(strip_ansi("plain"), "plain");
        assert_eq!(strip_ansi("a\x1b]0;title\x07b"), "ab");
        assert_eq!(strip_ansi("trailing\x1b"), "trailing");
    }

    fn rec() -> Recorder {
        Recorder::new(&[], &BTreeMap::new(), &BTreeMap::new()).0
    }

    #[test]
    fn reads_the_hud_line_of_the_game() {
        let r = rec();
        assert!(r.feed(
            "DexNom Runtime | 1 fps 1735.7 ms/frame | gpu 0% busy cpu 52% ram 11061 MB | 9 draws/frame | up 1:38"
        ));
        let l = r.latest();
        assert_eq!(l["fps"], 1.0, "not the 1735.7 that follows");
        assert_eq!(l["frame_ms"], 1735.7);
        assert_eq!((l["gpu"], l["cpu"], l["ram"], l["draws"]), (0.0, 52.0, 11061.0, 9.0));
    }

    #[test]
    fn frame_time_comes_from_the_hud_line_only() {
        let r = rec();
        // The copy statistics of the real log say `ms/frame` too.
        r.feed("INFO victory_gpu::copystat:   copied from 0x3a5eff80: 0.0x/frame, 1.45 MB/frame, 0.44 ms/frame");
        assert!(!r.latest().contains_key("frame_ms"));
        r.feed("INFO victory_gpu::host: 30.0 fps, 33.3 ms/frame (1.9 preparing), 3 draws/frame, cpu 55.5% gpu 0% busy ram 5696 MB");
        let l = r.latest();
        assert_eq!(
            (l["fps"], l["frame_ms"], l["draws"], l["cpu"], l["gpu"], l["ram"]),
            (30.0, 33.3, 3.0, 55.5, 0.0, 5696.0)
        );
        r.feed("DexNom Runtime | 3 fps 333.3 ms/frame | gpu 1% busy");
        assert_eq!(r.latest()["frame_ms"], 333.3);
    }

    #[test]
    fn phases_follow_the_output_and_the_hand() {
        let phases = BTreeMap::from([
            ("licences".to_string(), r#"file opened path="G:.{1,4}pack.{1,4}core_licenses"#.to_string()),
            ("histoire".to_string(), "story000".to_string()),
        ]);
        let (r, bad) = Recorder::new(&[], &BTreeMap::new(), &phases);
        assert!(bad.is_empty());
        assert_eq!(r.phase(), None);
        r.feed(r#"INFO dstorage: file opened path="G:\\pack\\core_licenses\\core_licenses.rpf""#);
        assert_eq!(r.phase().as_deref(), Some("licences"));
        r.feed("again core_licenses");
        r.mark("menu (A)");
        r.feed("opening story000");
        r.push("proc_cpu", 1.0);
        let s = r.finish("id".into(), "b", "", "pc").unwrap();
        let names: Vec<&str> = s.phases.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["licences", "menu (A)", "histoire"]);
    }

    #[test]
    fn reads_the_usual_fps_formats() {
        for (line, fps) in [("stats fps: 59.9 ms=16", 59.9), ("FPS=30", 30.0), ("running at 120 fps", 120.0)] {
            let r = rec();
            assert!(r.feed(line), "{line}");
            assert_eq!(r.latest()["fps"], fps);
        }
        let r = rec();
        assert!(!r.feed("shader translated stage=pixel"));
        assert!(!r.feed("fps: 0"));
        assert!(!r.feed("fps: 99999"));
    }

    #[test]
    fn samples_are_spaced_and_summarized() {
        let r = rec();
        r.feed("fps: 60");
        r.feed("fps: 61");
        r.push("proc_cpu", 12.0);
        r.push("proc_cpu", 14.0);
        let s = r.finish("id".into(), "amd", "0123456789abcdef", "pc").unwrap();
        assert_eq!((s.samples, s.avg, s.commit.as_str()), (1, 60.0, "0123456789"), "fps too close to the first");
        assert_eq!(s.metrics["proc_cpu"].samples, 2);
        assert!(rec().finish("id".into(), "amd", "", "pc").is_none());
        // Without FPS but with system numbers the run is still kept.
        let r = rec();
        r.push("sys_ram", 100.0);
        let s = r.finish("id".into(), "amd", "", "pc").unwrap();
        assert_eq!(s.samples, 0);
    }

    #[test]
    fn custom_patterns_replace_the_defaults() {
        let custom = BTreeMap::from([("vram".to_string(), r"vram=(\d+)".to_string()), ("bad".into(), "(".into())]);
        let (r, bad) = Recorder::new(&[r"frames/s=(\d+)".into(), "no group".into()], &custom, &BTreeMap::new());
        assert_eq!(bad.len(), 2);
        assert!(r.feed("frames/s=45 vram=800"));
        assert_eq!((r.latest()["fps"], r.latest()["vram"]), (45.0, 800.0));
        assert!(!r.feed("fps: 60"));
    }
}
