//! Git through the `git` program, so the user's credentials, proxies and
//! long-path settings apply as they do in a terminal.

use std::path::Path;
use std::process::Stdio;

use anyhow::{Context, bail};
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use crate::process::{self, Cmd, LogSink};

#[derive(Debug, Clone, Default, Serialize)]
pub struct Commit {
    pub hash: String,
    pub short: String,
    pub subject: String,
    pub author: String,
    pub time: i64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct GitStatus {
    pub cloned: bool,
    pub head: Option<Commit>,
    /// Local branch, or none when detached (tag, commit).
    pub branch: Option<String>,
    /// Tracked files changed locally.
    pub dirty: bool,
    /// Commits on the remote ref not checked out yet.
    pub behind: u32,
    /// Local commits not on the remote ref.
    pub ahead: u32,
    /// The newest incoming commits (at most 30).
    pub incoming: Vec<Commit>,
    /// The ref is a branch on the remote (pull applies) rather than a tag or commit.
    pub ref_is_branch: bool,
    pub remote_url: Option<String>,
}

fn git_cmd(args: &[&str], cwd: &Path) -> Cmd {
    let mut argv: Vec<String> = vec!["git".into(), "-c".into(), "advice.detachedHead=false".into()];
    if cfg!(windows) {
        argv.extend(["-c".into(), "core.longpaths=true".into()]);
    }
    argv.extend(args.iter().map(|s| s.to_string()));
    let mut cmd = Cmd::new(&argv, cwd).expect("non-empty");
    cmd.env.insert("GIT_TERMINAL_PROMPT".into(), "0".into());
    cmd
}

/// Runs git and returns its stdout (for queries).
async fn output(args: &[&str], cwd: &Path) -> anyhow::Result<String> {
    let cmd = git_cmd(args, cwd);
    let mut c = tokio::process::Command::new(crate::prereq::which("git").context("git not found on PATH")?);
    if let Some(path) = crate::prereq::current_path() {
        c.env("PATH", path);
    }
    c.args(&cmd.args).current_dir(cwd).envs(&cmd.env).stdin(Stdio::null());
    #[cfg(windows)]
    c.creation_flags(0x0800_0000);
    let out = c.output().await?;
    if !out.status.success() {
        bail!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

async fn ok(args: &[&str], cwd: &Path) -> bool {
    output(args, cwd).await.is_ok()
}

async fn run(args: &[&str], cwd: &Path, sink: &LogSink, cancel: &CancellationToken) -> anyhow::Result<()> {
    process::run(&git_cmd(args, cwd), sink.clone(), cancel).await
}

const LOG_FORMAT: &str = "--format=%H%x1f%h%x1f%s%x1f%an%x1f%ct";

fn parse_commits(text: &str) -> Vec<Commit> {
    text.lines()
        .filter_map(|l| {
            let mut p = l.split('\x1f');
            Some(Commit {
                hash: p.next()?.into(),
                short: p.next()?.into(),
                subject: p.next()?.into(),
                author: p.next()?.into(),
                time: p.next()?.parse().unwrap_or(0),
            })
        })
        .collect()
}

/// Branches and tags of a remote repository.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct RemoteRefs {
    pub branches: Vec<String>,
    pub tags: Vec<String>,
}

/// Reads the output of `git ls-remote --heads --tags --refs`.
pub fn parse_ls_remote(text: &str) -> RemoteRefs {
    let mut out = RemoteRefs::default();
    for line in text.lines() {
        let Some((_, name)) = line.split_once('\t') else { continue };
        if let Some(b) = name.strip_prefix("refs/heads/") {
            out.branches.push(b.to_string());
        } else if let Some(t) = name.strip_prefix("refs/tags/") {
            out.tags.push(t.to_string());
        }
    }
    out.branches.sort();
    // Newest-looking tags first (v1.10 before v1.9).
    out.tags.sort_by(|a, b| natural_cmp(b, a));
    out
}

fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let key = |s: &str| -> Vec<(u64, String)> {
        s.split(|c: char| !c.is_ascii_alphanumeric())
            .map(|p| (p.parse::<u64>().unwrap_or(u64::MAX), p.to_string()))
            .collect()
    };
    key(a).cmp(&key(b))
}

/// The branches and tags of `repo`, without cloning it.
pub async fn remote_refs(repo: &str) -> anyhow::Result<RemoteRefs> {
    let text = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        output(&["ls-remote", "--heads", "--tags", "--refs", repo], &std::env::temp_dir()),
    )
    .await
    .context("the repository did not answer in 30 s")??;
    Ok(parse_ls_remote(&text))
}

/// The newest commits of `git_ref` in a clone (the remote branch when there is one).
pub async fn recent_commits(dir: &Path, git_ref: &str, n: usize) -> Vec<Commit> {
    let remote = format!("origin/{git_ref}");
    let target = if ok(&["rev-parse", "--verify", "-q", &format!("refs/remotes/{remote}")], dir).await {
        remote
    } else {
        git_ref.to_string()
    };
    let max = format!("-{n}");
    output(&["log", &max, LOG_FORMAT, &target], dir).await.map(|t| parse_commits(&t)).unwrap_or_default()
}

pub fn is_cloned(dir: &Path) -> bool {
    dir.join(".git").exists()
}

/// State of a checkout, using what was last fetched (no network).
pub async fn status(dir: &Path, git_ref: &str) -> anyhow::Result<GitStatus> {
    if !is_cloned(dir) {
        return Ok(GitStatus::default());
    }
    let mut s = GitStatus { cloned: true, ..Default::default() };
    s.head = parse_commits(&output(&["log", "-1", LOG_FORMAT], dir).await.unwrap_or_default()).pop();
    s.branch = output(&["symbolic-ref", "-q", "--short", "HEAD"], dir).await.ok().filter(|b| !b.is_empty());
    s.dirty = !output(&["status", "--porcelain", "--untracked-files=no"], dir).await?.is_empty();
    s.remote_url = output(&["remote", "get-url", "origin"], dir).await.ok();
    let remote = format!("refs/remotes/origin/{git_ref}");
    s.ref_is_branch = ok(&["rev-parse", "--verify", "-q", &remote], dir).await;
    let target = if s.ref_is_branch { remote } else { format!("{git_ref}^{{commit}}") };
    if ok(&["rev-parse", "--verify", "-q", &target], dir).await {
        let range = format!("HEAD...{target}");
        if let Ok(counts) = output(&["rev-list", "--left-right", "--count", &range], dir).await {
            let mut it = counts.split_whitespace().map(|n| n.parse().unwrap_or(0));
            s.ahead = it.next().unwrap_or(0);
            s.behind = it.next().unwrap_or(0);
        }
        if s.behind > 0 {
            let range = format!("HEAD..{target}");
            s.incoming =
                parse_commits(&output(&["log", "-n", "30", LOG_FORMAT, &range], dir).await.unwrap_or_default());
        }
    }
    Ok(s)
}

pub async fn fetch(dir: &Path, sink: &LogSink, cancel: &CancellationToken) -> anyhow::Result<()> {
    run(&["fetch", "--prune", "--tags", "--force", "--progress", "origin"], dir, sink, cancel).await
}

/// Clones `repo` into `dir` and checks out `git_ref`.
pub async fn clone(
    repo: &str,
    git_ref: &str,
    dir: &Path,
    sink: &LogSink,
    cancel: &CancellationToken,
) -> anyhow::Result<()> {
    if is_cloned(dir) {
        return Ok(());
    }
    let parent = dir.parent().context("invalid clone path")?;
    std::fs::create_dir_all(parent)?;
    if dir.exists() {
        // A clone that failed half way.
        std::fs::remove_dir_all(dir)?;
    }
    let dir_str = dir.to_string_lossy().to_string();
    if let Err(e) = run(&["clone", "--progress", "--no-checkout", "--", repo, &dir_str], parent, sink, cancel).await {
        let _ = std::fs::remove_dir_all(dir);
        return Err(e);
    }
    checkout(dir, git_ref, true, sink, cancel).await
}

/// Puts the work tree on `git_ref`. With `force`, local changes to tracked
/// files are discarded and the branch is reset to the remote.
async fn checkout(
    dir: &Path,
    git_ref: &str,
    force: bool,
    sink: &LogSink,
    cancel: &CancellationToken,
) -> anyhow::Result<()> {
    let remote = format!("origin/{git_ref}");
    let is_branch = ok(&["rev-parse", "--verify", "-q", &format!("refs/remotes/{remote}")], dir).await;
    if is_branch {
        if force {
            run(&["checkout", "--force", "-B", git_ref, &remote], dir, sink, cancel).await?;
            run(&["reset", "--hard", &remote], dir, sink, cancel).await
        } else {
            let current = output(&["symbolic-ref", "-q", "--short", "HEAD"], dir).await.unwrap_or_default();
            if current != git_ref {
                if ok(&["rev-parse", "--verify", "-q", &format!("refs/heads/{git_ref}")], dir).await {
                    run(&["checkout", git_ref], dir, sink, cancel).await?;
                } else {
                    run(&["checkout", "-b", git_ref, "--track", &remote], dir, sink, cancel).await?;
                }
            }
            run(&["merge", "--ff-only", &remote], dir, sink, cancel).await
        }
    } else if ok(&["rev-parse", "--verify", "-q", &format!("{git_ref}^{{commit}}")], dir).await {
        let mut args = vec!["checkout", "--detach"];
        if force {
            args.push("--force");
        }
        args.push(git_ref);
        run(&args, dir, sink, cancel).await
    } else {
        bail!("`{git_ref}` is not a branch, a tag or a commit of the repository")
    }
}

/// `git pull`: fetch, then fast-forward. Refuses when it would lose local
/// work (use `resync`).
pub async fn pull(dir: &Path, git_ref: &str, sink: &LogSink, cancel: &CancellationToken) -> anyhow::Result<()> {
    fetch(dir, sink, cancel).await?;
    checkout(dir, git_ref, false, sink, cancel).await
}

/// Forces the checkout to match the server exactly: fetch, reset, and
/// remove untracked files. Ignored files (build output such as `target/`)
/// are kept so the next build stays incremental.
pub async fn resync(dir: &Path, git_ref: &str, sink: &LogSink, cancel: &CancellationToken) -> anyhow::Result<()> {
    fetch(dir, sink, cancel).await?;
    checkout(dir, git_ref, true, sink, cancel).await?;
    run(&["clean", "-fd"], dir, sink, cancel).await
}

/// Makes sure `dir` is a clone of `repo` (the URL may have changed on the
/// server).
pub async fn ensure_remote(dir: &Path, repo: &str, sink: &LogSink, cancel: &CancellationToken) -> anyhow::Result<()> {
    let current = output(&["remote", "get-url", "origin"], dir).await.unwrap_or_default();
    if current != repo {
        run(&["remote", "set-url", "origin", repo], dir, sink, cancel).await?;
    }
    Ok(())
}

pub async fn head(dir: &Path) -> anyhow::Result<String> {
    output(&["rev-parse", "HEAD"], dir).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_ls_remote() {
        let text =
            "aaa\trefs/heads/main\nbbb\trefs/heads/dev\nccc\trefs/tags/v1.9\nddd\trefs/tags/v1.10\nignored line\n";
        let r = parse_ls_remote(text);
        assert_eq!(r.branches, ["dev", "main"]);
        assert_eq!(r.tags, ["v1.10", "v1.9"], "newest first, numbers compared as numbers");
    }
}
