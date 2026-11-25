use std::time::Duration;

use anyhow::{Context, bail};
use oedvi_common::{BuilderList, Manifest, ScanStatus};
use reqwest::{StatusCode, Url};
use serde::Deserialize;

/// HTTP client of oedvi-server.
#[derive(Clone)]
pub struct Api {
    base: Url,
    token: String,
    http: reqwest::Client,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    error: String,
}

impl Api {
    pub fn new(server_url: &str, token: &str) -> anyhow::Result<Self> {
        let mut base = Url::parse(server_url.trim()).with_context(|| format!("invalid server URL {server_url:?}"))?;
        if !base.path().ends_with('/') {
            base.set_path(&format!("{}/", base.path()));
        }
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(60))
            .user_agent(concat!("oedvi/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self { base, token: token.to_string(), http })
    }

    fn url(&self, path: &str) -> Url {
        self.base.join(path).expect("static path")
    }

    /// URL of a game file, each path segment escaped.
    pub fn file_url(&self, path: &str) -> Url {
        let mut url = self.url("files/");
        url.path_segments_mut().expect("http URL").pop_if_empty().extend(path.split('/'));
        url
    }

    pub fn request(&self, method: reqwest::Method, url: Url) -> reqwest::RequestBuilder {
        let req = self.http.request(method, url);
        if self.token.is_empty() { req } else { req.bearer_auth(&self.token) }
    }

    async fn check(resp: reqwest::Response) -> anyhow::Result<reqwest::Response> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let text = resp.text().await.unwrap_or_default();
        let msg = serde_json::from_str::<ErrorBody>(&text).map(|b| b.error).unwrap_or(text);
        match status {
            StatusCode::UNAUTHORIZED => bail!("the server refused the token ({msg})"),
            _ => bail!("{status}: {msg}"),
        }
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> anyhow::Result<T> {
        let resp = self.request(reqwest::Method::GET, self.url(path)).send().await.context("server unreachable")?;
        Ok(Self::check(resp).await?.json().await?)
    }

    pub async fn health(&self) -> anyhow::Result<serde_json::Value> {
        self.get_json("api/health").await
    }

    pub async fn status(&self) -> anyhow::Result<ScanStatus> {
        self.get_json("api/status").await
    }

    pub async fn manifest(&self) -> anyhow::Result<Manifest> {
        let resp = self
            .request(reqwest::Method::GET, self.url("api/manifest"))
            .timeout(Duration::from_secs(300))
            .send()
            .await
            .context("server unreachable")?;
        Ok(Self::check(resp).await?.json().await?)
    }

    pub async fn builders(&self) -> anyhow::Result<BuilderList> {
        self.get_json("api/builders").await
    }

    pub async fn list_ids(&self, kind: &str) -> anyhow::Result<Vec<String>> {
        self.get_json(&format!("api/ids/{kind}")).await
    }

    pub async fn read_raw(&self, kind: &str, id: &str) -> anyhow::Result<Option<String>> {
        let resp = self.request(reqwest::Method::GET, self.url(&format!("api/{kind}/{id}"))).send().await?;
        if resp.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        Ok(Some(Self::check(resp).await?.text().await?))
    }

    pub async fn write_raw(&self, kind: &str, id: &str, text: &str) -> anyhow::Result<BuilderList> {
        let resp = self
            .request(reqwest::Method::PUT, self.url(&format!("api/{kind}/{id}")))
            .body(text.to_string())
            .send()
            .await?;
        Ok(Self::check(resp).await?.json().await?)
    }

    pub async fn check_raw(&self, kind: &str, id: &str, text: &str) -> anyhow::Result<BuilderList> {
        let resp = self
            .request(reqwest::Method::POST, self.url(&format!("api/{kind}/{id}/check")))
            .body(text.to_string())
            .send()
            .await?;
        Ok(Self::check(resp).await?.json().await?)
    }

    pub async fn delete_raw(&self, kind: &str, id: &str) -> anyhow::Result<()> {
        let resp = self.request(reqwest::Method::DELETE, self.url(&format!("api/{kind}/{id}"))).send().await?;
        Self::check(resp).await?;
        Ok(())
    }

    /// Sends the FPS of a run to the server.
    pub async fn post_session(&self, s: &oedvi_common::stats::Session) -> anyhow::Result<()> {
        let resp = self.request(reqwest::Method::POST, self.url("api/stats")).json(s).send().await?;
        Self::check(resp).await?;
        Ok(())
    }

    /// Every builder and version compared, with the newest runs.
    pub async fn stats(&self) -> anyhow::Result<oedvi_common::stats::StatsReport> {
        self.get_json("api/stats").await
    }

    pub async fn rescan(&self) -> anyhow::Result<()> {
        let resp = self.request(reqwest::Method::POST, self.url("api/rescan")).send().await?;
        Self::check(resp).await?;
        Ok(())
    }

    /// Downloads `[start, end)` of a game file.
    pub async fn fetch_range(&self, path: &str, start: u64, end: u64) -> anyhow::Result<Vec<u8>> {
        use futures_util::StreamExt;
        let resp = self
            .request(reqwest::Method::GET, self.file_url(path))
            .header(reqwest::header::RANGE, format!("bytes={}-{}", start, end - 1))
            .send()
            .await?;
        let status = resp.status();
        let resp = Self::check(resp).await?;
        let len = (end - start) as usize;
        if status != StatusCode::PARTIAL_CONTENT && start != 0 {
            bail!("the server does not support range requests (HTTP {status})");
        }
        let mut buf = Vec::with_capacity(len);
        let mut stream = resp.bytes_stream();
        while let Some(part) = stream.next().await {
            let part = part?;
            let take = part.len().min(len - buf.len());
            buf.extend_from_slice(&part[..take]);
            if buf.len() == len {
                break;
            }
        }
        if buf.len() != len {
            bail!("truncated response ({} bytes of {len})", buf.len());
        }
        Ok(buf)
    }
}
