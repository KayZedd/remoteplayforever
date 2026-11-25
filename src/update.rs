//! Looks up the newest GitHub release and tells the window when it is newer than this build.
//!
//! One anonymous GET to the public releases API, no identifiers sent, switchable in Settings.

use serde::Deserialize;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

const LATEST_RELEASE_API: &str =
    "https://api.github.com/repos/KayZedd/remoteplayforever/releases/latest";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInfo {
    pub version: String,
    pub url: String,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

/// `v1.2.3` or `1.2.3` as (major, minor, patch). Pre-release suffixes (`-rc1`) are rejected.
fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.trim().trim_start_matches('v').split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some((major, minor, patch))
}

/// The release described by the API response, if it is a final release newer than `current`.
fn newer_release(json: &str, current: &str) -> Option<UpdateInfo> {
    let release: Release = serde_json::from_str(json).ok()?;
    if release.draft || release.prerelease {
        return None;
    }
    let latest = parse_version(&release.tag_name)?;
    (latest > parse_version(current)?).then(|| UpdateInfo {
        version: release.tag_name.trim_start_matches('v').to_string(),
        url: release.html_url,
    })
}

fn fetch_latest() -> Option<String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(8)))
        .build()
        .into();
    agent
        .get(LATEST_RELEASE_API)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "RemotePlayForever-update-check")
        .call()
        .ok()?
        .body_mut()
        .read_to_string()
        .ok()
}

/// Starts the lookup on a background thread. The receiver yields one message, only when an update exists.
pub fn check_in_background() -> Receiver<UpdateInfo> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        if let Some(json) = fetch_latest()
            && let Some(info) = newer_release(&json, env!("CARGO_PKG_VERSION"))
        {
            let _ = tx.send(info);
        }
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, extra: &str) -> String {
        format!(r#"{{"tag_name":"{tag}","html_url":"https://example.test/r","name":"x"{extra}}}"#)
    }

    #[test]
    fn parses_tags_with_and_without_v() {
        assert_eq!(parse_version("v0.1.1"), Some((0, 1, 1)));
        assert_eq!(parse_version("1.20.3"), Some((1, 20, 3)));
    }

    #[test]
    fn rejects_malformed_versions() {
        for bad in ["", "v1", "1.2", "1.2.3.4", "1.2.x", "v1.2.3-rc1", "latest"] {
            assert_eq!(parse_version(bad), None, "{bad}");
        }
    }

    #[test]
    fn compares_numerically_not_as_text() {
        assert!(newer_release(&release("v0.10.0", ""), "0.9.0").is_some());
        assert!(newer_release(&release("v1.0.0", ""), "0.99.99").is_some());
    }

    #[test]
    fn same_or_older_is_not_an_update() {
        assert!(newer_release(&release("v0.1.1", ""), "0.1.1").is_none());
        assert!(newer_release(&release("v0.1.0", ""), "0.1.1").is_none());
    }

    #[test]
    fn reports_version_and_url() {
        assert_eq!(
            newer_release(&release("v0.2.0", ""), "0.1.1"),
            Some(UpdateInfo {
                version: "0.2.0".into(),
                url: "https://example.test/r".into()
            })
        );
    }

    #[test]
    fn drafts_and_prereleases_are_ignored() {
        assert!(newer_release(&release("v9.0.0", r#","draft":true"#), "0.1.1").is_none());
        assert!(newer_release(&release("v9.0.0", r#","prerelease":true"#), "0.1.1").is_none());
    }

    #[test]
    fn garbage_response_is_not_an_update() {
        assert!(newer_release("not json", "0.1.1").is_none());
        assert!(newer_release(r#"{"message":"rate limited"}"#, "0.1.1").is_none());
    }
}
