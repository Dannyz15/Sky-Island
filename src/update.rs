//! Checking whether a newer version has been released on GitHub, and downloading its installer. Both
//! only happen when the user presses the button in the settings window. Nothing is installed until
//! the user presses Install.

use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use serde_json::Value;
use sha2::{Digest, Sha256};
use winit::event_loop::EventLoopProxy;

use crate::UserEvent;

/// Where this project's releases live. The release page and the installer must both be on it.
const REPO: &str = "https://github.com/Dannyz15/Sky-Island/";
const LATEST: &str = "https://api.github.com/repos/Dannyz15/Sky-Island/releases/latest";
const RELEASES: &str = "https://api.github.com/repos/Dannyz15/Sky-Island/releases?per_page=8";

/// The installer attached to a release.
#[derive(Clone, PartialEq, Debug)]
pub struct Asset {
    pub url: String,
    pub size: u64,
    /// Lower-case SHA-256 that GitHub reports for the file. An installer without one is not offered.
    pub sha256: String,
}

/// How a check for updates stands.
#[derive(Clone, PartialEq, Debug)]
pub enum Status {
    /// Not checked yet.
    Idle,
    Checking,
    /// This is the newest version.
    Latest,
    /// A newer version exists. `page` is its release page; `asset` is the installer, if it has one.
    Available { version: String, page: String, asset: Option<Asset> },
    /// The installer is coming down.
    Downloading { version: String, percent: u32 },
    /// The installer is on disk and checked, waiting for the user to press Install.
    Ready { version: String, path: PathBuf },
    /// The check did not work, for example because there is no internet.
    Failed,
    DownloadFailed,
}

/// One release and its notes, for the changelog.
#[derive(Clone, PartialEq, Debug)]
pub struct Release {
    pub version: String,
    /// "2026-10-29".
    pub date: String,
    pub body: String,
}

/// How loading the changelog stands.
#[derive(Clone, PartialEq, Debug)]
pub enum Changelog {
    Idle,
    Loading,
    Ready(Vec<Release>),
    Failed,
}

/// Fetch the latest releases and their notes on a background thread; they arrive as `UserEvent::Changelog`.
pub fn spawn_changelog(proxy: EventLoopProxy<UserEvent>) {
    thread::spawn(move || {
        let _ = proxy.send_event(UserEvent::Changelog(fetch_changelog()));
    });
}

fn fetch_changelog() -> Option<Vec<Release>> {
    let json = agent(10)
        .get(RELEASES)
        .header("User-Agent", concat!("Sky-Island/", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/vnd.github+json")
        .call()
        .ok()?
        .body_mut()
        .read_json::<Value>()
        .ok()?;
    let releases = releases_from(&json)?;
    (!releases.is_empty()).then_some(releases)
}

/// The releases in GitHub's answer, newest first as GitHub lists them.
fn releases_from(json: &Value) -> Option<Vec<Release>> {
    let list = json.as_array()?;
    Some(
        list.iter()
            .filter(|r| !r.get("draft").and_then(Value::as_bool).unwrap_or(false))
            .filter_map(|r| {
                let tag = r.get("tag_name")?.as_str()?;
                Some(Release {
                    version: tag.trim_start_matches(['v', 'V']).to_string(),
                    date: r.get("published_at").and_then(Value::as_str).unwrap_or("").chars().take(10).collect(),
                    body: r.get("body").and_then(Value::as_str).unwrap_or("").to_string(),
                })
            })
            .collect(),
    )
}

/// A line of release notes, with the Markdown taken off.
#[derive(Clone, PartialEq, Debug)]
pub enum Note {
    Heading(String),
    Bullet(String),
    Text(String),
}

/// Remove the Markdown marks that would show as clutter: bold, code ticks, and links down to their text.
pub fn plain(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' | '`' => {}
            '[' => {
                // [text](address) becomes text.
                let label: String = chars.by_ref().take_while(|c| *c != ']').collect();
                if chars.peek() == Some(&'(') {
                    for c in chars.by_ref() {
                        if c == ')' {
                            break;
                        }
                    }
                }
                out.push_str(&label);
            }
            _ => out.push(c),
        }
    }
    out.trim().to_string()
}

/// The notes of a release as headings, bullets and plain lines. Titles, rules, tables and the
/// "Full changelog" link are left out.
pub fn notes_of(body: &str) -> Vec<Note> {
    let mut notes = Vec::new();
    for raw in body.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("# ") || line.starts_with("---") || line.starts_with('|') || line.starts_with("**Full changelog") {
            continue;
        }
        let note = if let Some(h) = line.strip_prefix("### ").or_else(|| line.strip_prefix("## ")) {
            Note::Heading(plain(h))
        } else if let Some(b) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            Note::Bullet(plain(b))
        } else {
            Note::Text(plain(line.trim_start_matches("> ")))
        };
        notes.push(note);
    }
    notes
}

/// Ask GitHub on a background thread; the answer arrives as `UserEvent::Update`.
pub fn spawn_check(proxy: EventLoopProxy<UserEvent>) {
    thread::spawn(move || {
        let _ = proxy.send_event(UserEvent::Update(check()));
    });
}

fn agent(seconds: u64) -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(seconds))).build().into()
}

fn check() -> Status {
    let json = agent(8)
        .get(LATEST)
        .header("User-Agent", concat!("Sky-Island/", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/vnd.github+json")
        .call()
        .ok()
        .and_then(|mut r| r.body_mut().read_json::<Value>().ok());
    json.as_ref().and_then(|v| judge(v, env!("CARGO_PKG_VERSION"))).unwrap_or(Status::Failed)
}

/// Read GitHub's answer for the latest release and compare it with the running version.
fn judge(release: &Value, current: &str) -> Option<Status> {
    let tag = release.get("tag_name")?.as_str()?;
    let version = tag.trim_start_matches(['v', 'V']);
    if newer(version, current) {
        let page = release.get("html_url").and_then(Value::as_str).filter(|u| u.starts_with(REPO)).unwrap_or("https://github.com/Dannyz15/Sky-Island/releases/latest");
        Some(Status::Available { version: version.to_string(), page: page.to_string(), asset: installer(release) })
    } else {
        parse(version).map(|_| Status::Latest)
    }
}

/// The setup program among a release's files.
fn installer(release: &Value) -> Option<Asset> {
    release.get("assets")?.as_array()?.iter().find_map(|a| {
        let name = a.get("name")?.as_str()?;
        if !(name.starts_with("Sky-Island-Setup") && name.ends_with(".exe")) {
            return None;
        }
        // Only a file on this project's own release downloads, with a size and a SHA-256 to check it by.
        let url = a.get("browser_download_url")?.as_str()?;
        let size = a.get("size").and_then(Value::as_u64).filter(|s| *s > 0)?;
        let sha256 = a.get("digest").and_then(Value::as_str)?.strip_prefix("sha256:")?.to_lowercase();
        let hex = sha256.len() == 64 && sha256.bytes().all(|b| b.is_ascii_hexdigit());
        (url.starts_with(&format!("{REPO}releases/download/")) && hex).then(|| Asset { url: url.to_string(), size, sha256 })
    })
}

/// "1.2.3" as numbers; `None` if it is not a plain dotted version.
fn parse(version: &str) -> Option<Vec<u64>> {
    version.split('.').map(|p| p.parse().ok()).collect()
}

/// Whether `latest` is a higher version than `current`.
fn newer(latest: &str, current: &str) -> bool {
    let (Some(mut a), Some(mut b)) = (parse(latest), parse(current)) else { return false };
    let len = a.len().max(b.len());
    a.resize(len, 0);
    b.resize(len, 0);
    a > b
}

/// Download the installer on a background thread into the temporary folder. Progress and the end
/// result arrive as `UserEvent::Update`.
pub fn spawn_download(version: String, asset: Asset, proxy: EventLoopProxy<UserEvent>) {
    thread::spawn(move || {
        let send = |s: Status| {
            let _ = proxy.send_event(UserEvent::Update(s));
        };
        match download(&version, &asset, |percent| send(Status::Downloading { version: version.clone(), percent })) {
            Some(path) => send(Status::Ready { version: version.clone(), path }),
            None => send(Status::DownloadFailed),
        }
    });
}

fn download(version: &str, asset: &Asset, mut progress: impl FnMut(u32)) -> Option<PathBuf> {
    // The version only becomes part of a file name, so keep it to digits and dots.
    let clean: String = version.chars().filter(|c| c.is_ascii_digit() || *c == '.').collect();
    let dir = std::env::temp_dir();
    let part = dir.join(format!("Sky-Island-Setup-{clean}.part"));
    let done = dir.join(format!("Sky-Island-Setup-{clean}.exe"));

    let mut response = agent(900).get(&asset.url).header("User-Agent", concat!("Sky-Island/", env!("CARGO_PKG_VERSION"))).call().ok()?;
    let total = response.body().content_length().unwrap_or(asset.size).max(1);
    let mut reader = response.body_mut().as_reader();
    let mut file = fs::File::create(&part).ok()?;
    let mut hash = Sha256::new();
    let (mut got, mut last, mut buf) = (0u64, 0u32, vec![0u8; 64 * 1024]);
    progress(0);
    let finished = loop {
        let n = match reader.read(&mut buf) {
            Ok(n) => n,
            Err(_) => break false,
        };
        if n == 0 {
            break true;
        }
        if file.write_all(&buf[..n]).is_err() {
            break false;
        }
        hash.update(&buf[..n]);
        got += n as u64;
        let percent = (got * 100 / total).min(100) as u32;
        if percent != last {
            last = percent;
            progress(percent);
        }
    };
    drop(file);
    let hex: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if !finished || !acceptable(got, &hex, asset) {
        let _ = fs::remove_file(&part);
        return None;
    }
    let _ = fs::remove_file(&done);
    fs::rename(&part, &done).ok()?;
    Some(done)
}

/// Delete setup files that "Check for updates" left in the temporary folder once they are no longer
/// needed: the version they hold is the one running (or older), so the update has been installed, or
/// the download was cut short. A setup that is still running can't be deleted, so it is tried again
/// for a while; a newer setup that was downloaded but not installed is kept.
pub fn clean_up_downloads() {
    thread::spawn(|| {
        for _ in 0..12 {
            let Ok(entries) = fs::read_dir(std::env::temp_dir()) else { return };
            let mut left = false;
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if stale(&name, env!("CARGO_PKG_VERSION")) && fs::remove_file(entry.path()).is_err() {
                    left = true;
                }
            }
            if !left {
                return;
            }
            thread::sleep(Duration::from_secs(5));
        }
    });
}

/// Whether a file name in the temporary folder is a download of ours that is no longer needed.
fn stale(name: &str, current: &str) -> bool {
    let Some(rest) = name.strip_prefix("Sky-Island-Setup-") else { return false };
    if rest.ends_with(".part") {
        return true;
    }
    rest.strip_suffix(".exe").is_some_and(|version| parse(version).is_some() && !newer(version, current))
}

/// Whether a downloaded file is the one the release describes: the same size and the same hash.
fn acceptable(got: u64, sha256: &str, asset: &Asset) -> bool {
    asset.size > 0 && got == asset.size && asset.sha256 == sha256
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn compares_versions_number_by_number() {
        assert!(newer("1.2.0", "1.1.0"));
        assert!(newer("1.10.0", "1.9.0"), "10 is more than 9");
        assert!(newer("2.0", "1.9.9"));
        assert!(!newer("1.1.0", "1.1.0"));
        assert!(!newer("1.1", "1.1.0"), "missing parts count as zero");
        assert!(!newer("1.0.9", "1.1.0"));
        assert!(!newer("beta", "1.1.0"), "something that is not a version is never newer");
    }

    fn release() -> Value {
        json!({
            "tag_name": "v1.2.0",
            "html_url": "https://github.com/Dannyz15/Sky-Island/releases/tag/v1.2.0",
            "assets": [
                { "name": "notes.txt", "browser_download_url": "https://github.com/Dannyz15/Sky-Island/releases/download/v1.2.0/notes.txt", "size": 5 },
                { "name": "Sky-Island-Setup-1.2.0.exe", "browser_download_url": SETUP_URL, "size": 1234, "digest": DIGEST }
            ]
        })
    }

    const SETUP_URL: &str = "https://github.com/Dannyz15/Sky-Island/releases/download/v1.2.0/Sky-Island-Setup-1.2.0.exe";
    const DIGEST: &str = "sha256:ABCDEF0123456789abcdef0123456789ABCDEF0123456789abcdef0123456789";

    #[test]
    fn reads_a_release_answer_and_finds_the_installer() {
        let asset = Asset { url: SETUP_URL.into(), size: 1234, sha256: DIGEST.strip_prefix("sha256:").unwrap().to_lowercase() };
        let page = "https://github.com/Dannyz15/Sky-Island/releases/tag/v1.2.0";
        assert_eq!(judge(&release(), "1.1.0"), Some(Status::Available { version: "1.2.0".into(), page: page.into(), asset: Some(asset) }));
        assert_eq!(judge(&release(), "1.2.0"), Some(Status::Latest));
        assert_eq!(judge(&release(), "1.3.0"), Some(Status::Latest), "a newer local build is not an update");
        assert_eq!(judge(&json!({ "message": "Not Found" }), "1.1.0"), None);
    }

    #[test]
    fn a_release_without_an_installer_still_counts_as_an_update() {
        let bare = json!({ "tag_name": "v1.2.0", "html_url": "https://github.com/Dannyz15/Sky-Island/releases/tag/v1.2.0", "assets": [] });
        assert!(matches!(judge(&bare, "1.1.0"), Some(Status::Available { asset: None, .. })));
    }

    #[test]
    fn old_downloads_are_cleaned_up_but_a_newer_one_is_kept() {
        assert!(stale("Sky-Island-Setup-1.1.0.exe", "1.1.0"), "installed: this version is running");
        assert!(stale("Sky-Island-Setup-1.0.0.exe", "1.1.0"));
        assert!(stale("Sky-Island-Setup-1.2.0.part", "1.1.0"), "an unfinished download");
        assert!(!stale("Sky-Island-Setup-1.2.0.exe", "1.1.0"), "downloaded but not installed yet");
        assert!(!stale("Sky-Island-Setup-latest.exe", "1.1.0"), "not one of ours");
        assert!(!stale("Other-Setup-1.0.0.exe", "1.1.0"));
    }

    #[test]
    fn markdown_marks_are_taken_off() {
        assert_eq!(plain("**Smaller notch.** The closed notch is now `148 x 28`"), "Smaller notch. The closed notch is now 148 x 28");
        assert_eq!(plain("see [the page](https://example.com/x) now"), "see the page now");
        assert_eq!(plain("  plain  "), "plain");
    }

    #[test]
    fn release_notes_become_headings_bullets_and_text() {
        let body = "# Sky Island 1.3.0\n\n## What's new\n\n- **Smaller notch.** It is 148 x 28.\n- Eyes\n\n---\n\nA closing line.\n\n**Full changelog:** [x](y)";
        assert_eq!(
            notes_of(body),
            vec![
                Note::Heading("What's new".into()),
                Note::Bullet("Smaller notch. It is 148 x 28.".into()),
                Note::Bullet("Eyes".into()),
                Note::Text("A closing line.".into()),
            ]
        );
    }

    #[test]
    fn releases_are_read_from_githubs_list() {
        let json = json!([
            { "tag_name": "v1.3.0", "published_at": "2026-10-29T10:00:00Z", "body": "- a" },
            { "tag_name": "v1.2.0", "published_at": "2026-10-29T08:00:00Z", "body": null },
            { "tag_name": "v9.9.9", "draft": true, "body": "hidden" }
        ]);
        let releases = releases_from(&json).unwrap();
        assert_eq!(releases.len(), 2, "a draft is not shown");
        assert_eq!((releases[0].version.as_str(), releases[0].date.as_str()), ("1.3.0", "2026-10-29"));
        assert_eq!(releases[1].body, "", "a release with no notes has an empty body");
        assert!(releases_from(&json!({ "message": "Not Found" })).is_none());
    }

    #[test]
    fn a_download_is_accepted_only_if_size_and_hash_match() {
        let asset = Asset { url: String::new(), size: 10, sha256: "aa".into() };
        assert!(acceptable(10, "aa", &asset));
        assert!(!acceptable(9, "aa", &asset), "cut short");
        assert!(!acceptable(10, "bb", &asset), "different file");
        let no_size = Asset { url: String::new(), size: 0, sha256: "aa".into() };
        assert!(!acceptable(0, "aa", &no_size), "a size of zero is never enough");
    }

    #[test]
    fn an_installer_must_be_on_this_projects_releases_and_carry_a_hash() {
        let with = |url: &str, digest: Option<&str>, size: u64| {
            let mut asset = json!({ "name": "Sky-Island-Setup-1.2.0.exe", "browser_download_url": url, "size": size });
            if let Some(d) = digest {
                asset["digest"] = json!(d);
            }
            json!({ "tag_name": "v1.2.0", "html_url": "https://github.com/Dannyz15/Sky-Island/releases/tag/v1.2.0", "assets": [asset] })
        };
        assert!(installer(&with(SETUP_URL, Some(DIGEST), 1234)).is_some());
        assert!(installer(&with(SETUP_URL, None, 1234)).is_none(), "no hash: not offered");
        assert!(installer(&with(SETUP_URL, Some("sha256:abc"), 1234)).is_none(), "not a SHA-256");
        assert!(installer(&with(SETUP_URL, Some(DIGEST), 0)).is_none(), "no size");
        assert!(installer(&with("https://example.com/Sky-Island-Setup-1.2.0.exe", Some(DIGEST), 1234)).is_none(), "another site");
        assert!(installer(&with("https://github.com/Other/Repo/releases/download/v1/Sky-Island-Setup-1.2.0.exe", Some(DIGEST), 1234)).is_none(), "another project");
    }

    #[test]
    fn a_release_page_from_somewhere_else_is_replaced_by_the_projects_own() {
        let odd = json!({ "tag_name": "v1.2.0", "html_url": "file:///C:/Windows/System32/calc.exe", "assets": [] });
        let Some(Status::Available { page, .. }) = judge(&odd, "1.1.0") else { panic!("an update") };
        assert!(page.starts_with("https://github.com/Dannyz15/Sky-Island/"));
    }
}
