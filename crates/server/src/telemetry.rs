//! Access log, per-client download summaries and byte formatting.
//!
//! The access log goes to the `access` target, so it can be tuned on its own:
//! `RUST_LOG=info,access=warn` keeps only the refusals and errors, and
//! `RUST_LOG=info,access=debug` also shows every polling call and every
//! single chunk request.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::header;
use axum::middleware::Next;
use axum::response::Response;

use crate::api::Shared;

/// A client that was quiet this long is announced again when it comes back.
const CLIENT_IDLE: Duration = Duration::from_secs(10 * 60);
/// How often the downloads of the last period are summed up.
const SUMMARY_EVERY: Duration = Duration::from_secs(15);

/// Who the auth middleware decided the caller is. Set on the response so the
/// access log (which wraps auth) can print it.
#[derive(Clone, Copy)]
pub struct Role(pub &'static str);

#[derive(Default)]
struct Traffic {
    requests: u64,
    bytes: u64,
    last_path: String,
}

/// What the access log remembers between requests.
#[derive(Default)]
pub struct Activity {
    seen: Mutex<HashMap<IpAddr, Instant>>,
    downloads: Mutex<HashMap<IpAddr, Traffic>>,
}

impl Activity {
    /// `true` when this address has not been seen for a while.
    fn is_new(&self, ip: IpAddr) -> bool {
        let mut seen = self.seen.lock().unwrap();
        let now = Instant::now();
        let fresh = seen.get(&ip).is_none_or(|t| now.duration_since(*t) > CLIENT_IDLE);
        seen.insert(ip, now);
        fresh
    }

    fn record_download(&self, ip: IpAddr, bytes: u64, path: &str) {
        let mut map = self.downloads.lock().unwrap();
        let t = map.entry(ip).or_default();
        t.requests += 1;
        t.bytes += bytes;
        t.last_path = path.to_string();
    }

    fn take_downloads(&self) -> Vec<(IpAddr, Traffic)> {
        std::mem::take(&mut *self.downloads.lock().unwrap()).into_iter().collect()
    }
}

/// Logs one line per request. Game file requests are summed per client and
/// printed every few seconds instead, since a full sync is thousands of them.
pub async fn access(State(state): State<Shared>, req: Request, next: Next) -> Response {
    let started = Instant::now();
    let ip = req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip());
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let ranged = req.headers().contains_key(header::RANGE);
    // Only informative: anyone can send this header, so it is never trusted.
    let forwarded = req.headers().get("x-forwarded-for").and_then(|v| v.to_str().ok()).map(str::to_owned);

    let res = next.run(req).await;

    let status = res.status();
    let granted = res.extensions().get::<Role>().map_or("-", |r| r.0);
    // `denied:<why>` is for the refusal line; elsewhere the caller has no role.
    let (role, denied) = match granted.strip_prefix("denied:") {
        Some(why) => ("-", Some(why)),
        None => (granted, None),
    };
    let size =
        res.headers().get(header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
    let who = match (ip, &forwarded) {
        (Some(ip), Some(f)) => format!("{ip} (xff {f})"),
        (Some(ip), None) => ip.to_string(),
        (None, _) => "?".into(),
    };

    // Health checks carry no token, so only a granted role announces a client.
    if let Some(ip) = ip
        && role != "-"
        && state.activity.is_new(ip)
    {
        tracing::info!("client {who} connected as {role}");
    }

    let line = format!(
        "{who} {role} {method} {path} -> {} {}{} {}ms",
        status.as_u16(),
        size.map_or_else(|| "-".to_string(), bytes),
        if ranged { " (range)" } else { "" },
        started.elapsed().as_millis()
    );

    let is_file = path.starts_with("/files/") || path.starts_with("/launcher/");
    if is_file && status.is_success() {
        if let Some(ip) = ip {
            state.activity.record_download(ip, size.unwrap_or(0), &path);
        }
        tracing::debug!(target: "access", "{line}");
    } else if let Some(why) = denied {
        tracing::warn!(target: "access", "REFUSED {line}: {why}");
    } else if status.is_server_error() {
        tracing::error!(target: "access", "{line}");
    } else if status.is_client_error() {
        tracing::warn!(target: "access", "{line}");
    } else if path == "/api/health" || path == "/api/status" {
        // The launcher calls these every 15 seconds.
        tracing::debug!(target: "access", "{line}");
    } else {
        tracing::info!(target: "access", "{line}");
    }
    res
}

/// Prints what each client downloaded since the last tick.
pub fn spawn_download_summaries(activity: Arc<Activity>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(SUMMARY_EVERY);
        tick.tick().await;
        let mut last = Instant::now();
        loop {
            tick.tick().await;
            let secs = last.elapsed().as_secs_f64().max(0.001);
            last = Instant::now();
            for (ip, t) in activity.take_downloads() {
                tracing::info!(
                    "downloads {ip}: {} requests, {} in {:.0}s ({}/s), last {}",
                    t.requests,
                    bytes(t.bytes),
                    secs,
                    bytes((t.bytes as f64 / secs) as u64),
                    t.last_path
                );
            }
        }
    });
}

/// `45s`, `4m 10s`, `1h 23m`.
pub fn duration(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m {}s", secs / 60, secs % 60),
        _ => format!("{}h {}m", secs / 3600, secs % 3600 / 60),
    }
}

/// `1.5 GiB`, `320 MiB`, `12 KiB`, `800 B`.
pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{n} B") } else { format!("{v:.1} {}", UNITS[i]) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_sizes() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(1023), "1023 B");
        assert_eq!(bytes(1536), "1.5 KiB");
        assert_eq!(bytes(32 << 20), "32.0 MiB");
        assert_eq!(bytes(250 << 30), "250.0 GiB");
    }

    #[test]
    fn formats_durations() {
        assert_eq!(duration(5), "5s");
        assert_eq!(duration(250), "4m 10s");
        assert_eq!(duration(4980), "1h 23m");
    }

    #[test]
    fn announces_a_client_once() {
        let a = Activity::default();
        let ip: IpAddr = "203.0.113.7".parse().unwrap();
        assert!(a.is_new(ip));
        assert!(!a.is_new(ip));
    }

    #[test]
    fn sums_downloads_and_resets() {
        let a = Activity::default();
        let ip: IpAddr = "203.0.113.7".parse().unwrap();
        a.record_download(ip, 10, "/files/a");
        a.record_download(ip, 5, "/files/b");
        let taken = a.take_downloads();
        assert_eq!(taken.len(), 1);
        assert_eq!((taken[0].1.requests, taken[0].1.bytes, taken[0].1.last_path.as_str()), (2, 15, "/files/b"));
        assert!(a.take_downloads().is_empty());
    }
}
