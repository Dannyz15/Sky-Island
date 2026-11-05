//! Checking whether a newer version has been released on GitHub, and downloading its installer. The check
//! runs when the user presses the button in the settings window, and by itself at start and every few hours
//! unless the New version notice is switched off; the download only starts when the user asks for it. Nothing
//! is installed until the user presses Install.

use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
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
    /// The installer is on disk and checked, waiting for the user to press Install. `asset` is what it was
    /// checked against, to check it again when it is run.
    Ready { version: String, path: PathBuf, asset: Asset },
    /// The check did not work, for example because there is no internet.
    Failed,
    /// The newest release is not named exactly like `v1.2.3` (a stray dot, a space, a capital V, a missing
    /// number...), so it is not offered as an update.
    BadName,
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
                    version: release_version(tag).unwrap_or(tag.trim()).to_string(),
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

/// Whether the checks that nobody asked for are wanted (the New version notice is on).
static AUTO: AtomicBool = AtomicBool::new(false);
/// A little after start, so the first check does not compete with everything else that starts.
const AUTO_FIRST: Duration = Duration::from_secs(20);
const AUTO_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

/// Start looking for a new version by itself: once soon after start, then every few hours, while `on`. The
/// answers arrive as `UserEvent::AutoUpdate`. (Call once.)
pub fn spawn_auto(on: bool, proxy: EventLoopProxy<UserEvent>) {
    AUTO.store(on, Ordering::Relaxed);
    thread::spawn(move || {
        thread::sleep(AUTO_FIRST);
        loop {
            if AUTO.load(Ordering::Relaxed) {
                let _ = proxy.send_event(UserEvent::AutoUpdate(check()));
            }
            thread::sleep(AUTO_EVERY);
        }
    });
}

/// The notice was switched on or off. Switching it off stops the checks; switching it on looks at once.
pub fn set_auto(on: bool, proxy: &EventLoopProxy<UserEvent>) {
    if AUTO.swap(on, Ordering::Relaxed) == on || !on {
        return;
    }
    let proxy = proxy.clone();
    thread::spawn(move || {
        let _ = proxy.send_event(UserEvent::AutoUpdate(check()));
    });
}

/// What an answer to a check nobody asked for changes. It never disturbs a check or a download the user
/// started, and a failed check (no internet, a wrong name) is kept quiet: it changes nothing. Only a newer
/// version, or the news that this one is the newest, is taken.
pub fn take_auto(current: &Status, found: Status) -> Option<Status> {
    if matches!(current, Status::Checking | Status::Downloading { .. } | Status::Ready { .. }) {
        return None;
    }
    matches!(found, Status::Available { .. } | Status::Latest).then_some(found)
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

/// Read GitHub's answer for the latest release and compare it with the running version. A release is only
/// taken as an update if it is named exactly right: a tag that is not `v` and three numbers, or a setup whose
/// name and address do not say the same version as the tag, is not offered (see `release_version`, `installer`).
fn judge(release: &Value, current: &str) -> Option<Status> {
    let tag = release.get("tag_name")?.as_str()?;
    let Some(version) = release_version(tag) else { return Some(Status::BadName) };
    if newer(version, current) {
        let page = release.get("html_url").and_then(Value::as_str).filter(|u| u.starts_with(REPO)).unwrap_or("https://github.com/Dannyz15/Sky-Island/releases/latest");
        Some(Status::Available { version: version.to_string(), page: page.to_string(), asset: installer(release, version) })
    } else {
        Some(Status::Latest)
    }
}

/// The setup program of the release for `version`. It has to be named `Sky-Island-Setup-<version>.exe` and sit at
/// exactly `<this project>/releases/download/v<version>/Sky-Island-Setup-<version>.exe`, with a size and a SHA-256
/// to check it by, so a setup of another version attached by mistake is never offered.
fn installer(release: &Value, version: &str) -> Option<Asset> {
    let name = format!("Sky-Island-Setup-{version}.exe");
    let expected = format!("{REPO}releases/download/v{version}/{name}");
    release.get("assets")?.as_array()?.iter().find_map(|a| {
        if a.get("name")?.as_str()? != name {
            return None;
        }
        let url = a.get("browser_download_url")?.as_str()?;
        let size = a.get("size").and_then(Value::as_u64).filter(|s| *s > 0)?;
        let sha256 = a.get("digest").and_then(Value::as_str)?.strip_prefix("sha256:")?.to_lowercase();
        let hex = sha256.len() == 64 && sha256.bytes().all(|b| b.is_ascii_hexdigit());
        (url == expected && hex).then(|| Asset { url: url.to_string(), size, sha256 })
    })
}

/// The version a release tag names, only if the tag is written exactly right: a lower-case `v`, then three
/// numbers with dots between them and nothing else ("v1.4.8"). A stray dot ("v.1.4.8"), a space, a capital V, no
/// V, a fourth number, a suffix like "-beta" or a leading zero all make it not a version, so nothing is offered.
fn release_version(tag: &str) -> Option<&str> {
    let version = tag.strip_prefix('v')?;
    let parts: Vec<&str> = version.split('.').collect();
    let plain = |p: &str| !p.is_empty() && p.len() <= 9 && p.bytes().all(|b| b.is_ascii_digit()) && (p == "0" || !p.starts_with('0'));
    (parts.len() == 3 && parts.iter().all(|p| plain(p))).then_some(version)
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
            Some(path) => send(Status::Ready { version: version.clone(), path, asset: asset.clone() }),
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

    // A setup that an earlier run downloaded and left (it is only cleaned up once this version or a newer one is
    // installed): if it is exactly what the release describes, use it and do not fetch it again.
    if let Ok(file) = fs::File::open(&done)
        && matches(&file, asset)
    {
        drop(file);
        progress(100);
        return Some(done);
    }

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
        // More than the release says the setup is: it is not that setup, so stop before it fills the disk.
        if got > asset.size {
            break false;
        }
        let percent = (got * 100 / total).min(100) as u32;
        if percent != last {
            last = percent;
            progress(percent);
        }
    };
    drop(file);
    let hex = hex_of(&hash.finalize());
    if !finished || !acceptable(got, &hex, asset) {
        let _ = fs::remove_file(&part);
        return None;
    }
    let _ = fs::remove_file(&done);
    fs::rename(&part, &done).ok()?;
    Some(done)
}

/// Whether `file` holds the installer `asset` describes: the same size and the same SHA-256.
fn matches(mut file: impl Read, asset: &Asset) -> bool {
    let (mut hash, mut got, mut buf) = (Sha256::new(), 0u64, vec![0u8; 64 * 1024]);
    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                hash.update(&buf[..n]);
                got += n as u64;
            }
            Err(_) => return false,
        }
    }
    acceptable(got, &hex_of(&hash.finalize()), asset)
}

/// Run the downloaded installer, but only if the file is still exactly the one that was checked after the
/// download: it has been sitting in the temporary folder, where another program could have changed it. The file is
/// held open (nobody can write to it) from the check until the installer has started. A file that fails the check
/// is deleted. Returns whether the installer was started.
pub fn launch(path: &std::path::Path, asset: &Asset) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 1;
    let Ok(file) = fs::OpenOptions::new().read(true).share_mode(FILE_SHARE_READ).open(path) else { return false };
    if !matches(&file, asset) {
        drop(file);
        let _ = fs::remove_file(path);
        return false;
    }
    let started = std::process::Command::new(path).spawn().is_ok();
    drop(file);
    started
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

/// A checksum as the lower-case hex text GitHub gives it in.
fn hex_of(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether a downloaded file is the one the release describes: the same size and the same hash.
fn acceptable(got: u64, sha256: &str, asset: &Asset) -> bool {
    asset.size > 0 && got == asset.size && asset.sha256 == sha256
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset_of(bytes: &[u8]) -> Asset {
        Asset { url: String::new(), size: bytes.len() as u64, sha256: hex_of(&Sha256::digest(bytes)) }
    }

    #[test]
    fn a_file_matches_its_release_only_with_the_same_size_and_hash() {
        let bytes = b"the installer".as_slice();
        assert!(matches(bytes, &asset_of(bytes)));
        assert!(!matches(b"the installeR".as_slice(), &asset_of(bytes)), "one byte changed");
        assert!(!matches(b"the installer plus".as_slice(), &asset_of(bytes)), "longer");
        assert!(!matches(b"".as_slice(), &asset_of(bytes)), "empty");
    }

    #[test]
    fn an_installer_is_checked_again_before_it_is_run_and_a_changed_one_is_deleted() {
        // A real program stands in for the installer: it starts and ends at once.
        let system = std::env::var_os("SystemRoot").map(std::path::PathBuf::from).unwrap_or_else(|| "C:\\Windows".into());
        let original = fs::read(system.join("System32").join("whoami.exe")).unwrap();
        let dir = std::env::temp_dir().join(format!("sky-island-launch-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("Sky-Island-Setup-9.9.9.exe");
        fs::write(&file, &original).unwrap();
        let asset = asset_of(&original);

        assert!(launch(&file, &asset), "the file is as it was checked, and it can be started while it is held open");
        assert!(file.exists(), "a good installer is kept");

        let mut changed = original.clone();
        *changed.last_mut().unwrap() ^= 1;
        // The program that was just started keeps its file locked until it ends, which takes a moment.
        let mut written = false;
        for _ in 0..100 {
            written = fs::write(&file, &changed).is_ok();
            if written {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        assert!(written, "the started program ended and let go of its file");
        assert!(!launch(&file, &asset), "changed after the download: not run");
        assert!(!file.exists(), "and removed, so the download is offered again");
        assert!(!launch(&file, &asset), "a missing file is not run either");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_a_tag_written_exactly_as_v_and_three_numbers_is_a_version() {
        for good in ["v1.4.8", "v0.0.1", "v10.20.30"] {
            assert_eq!(release_version(good), Some(&good[1..]), "{good:?}");
        }
        for bad in [
            "", "v", "1.4.8", "V1.4.8", "v.1.4.8", "v1.4.8.", " v1.4.8", "v1.4.8 ", "v 1.4.8", "v1.4", "v1.4.8.1", "v1..8", "v1.4.x",
            "v1.4.8-beta", "v01.4.8", "v1.04.8", "v1,4,8", "vv1.4.8", "v1.4.8\n", "v1.4.1234567890", "v-1.4.8",
        ] {
            assert_eq!(release_version(bad), None, "{bad:?} must not be read as a version");
        }
    }

    #[test]
    fn a_mistyped_tag_is_never_offered_as_an_update() {
        for tag in ["v.1.4.9", "V1.4.9", "1.4.9", "v1.4.9 ", "v1.4"] {
            let release = json!({ "tag_name": tag, "html_url": "https://github.com/Dannyz15/Sky-Island/releases/tag/x", "assets": [] });
            assert_eq!(judge(&release, "1.4.8"), Some(Status::BadName), "{tag:?}");
        }
        let right = json!({ "tag_name": "v1.4.9", "html_url": "https://github.com/Dannyz15/Sky-Island/releases/tag/v1.4.9", "assets": [] });
        assert!(matches!(judge(&right, "1.4.8"), Some(Status::Available { version, .. }) if version == "1.4.9"));
        assert_eq!(judge(&right, "1.4.9"), Some(Status::Latest));
    }

    #[test]
    fn a_setup_of_another_version_attached_to_a_release_is_not_offered() {
        let attach = |name: &str, url: &str| {
            json!({
                "tag_name": "v1.2.0",
                "html_url": "https://github.com/Dannyz15/Sky-Island/releases/tag/v1.2.0",
                "assets": [{ "name": name, "browser_download_url": url, "size": 1234, "digest": DIGEST }]
            })
        };
        let base = "https://github.com/Dannyz15/Sky-Island/releases/download";
        let offered = |r: &Value| matches!(judge(r, "1.1.0"), Some(Status::Available { asset: Some(_), .. }));
        assert!(offered(&attach("Sky-Island-Setup-1.2.0.exe", &format!("{base}/v1.2.0/Sky-Island-Setup-1.2.0.exe"))));
        assert!(!offered(&attach("Sky-Island-Setup-1.1.9.exe", &format!("{base}/v1.2.0/Sky-Island-Setup-1.1.9.exe"))), "the setup of an older version");
        assert!(!offered(&attach("Sky-Island-Setup-1.2.0.exe", &format!("{base}/v1.1.9/Sky-Island-Setup-1.2.0.exe"))), "the address of another release");
        assert!(!offered(&attach("sky-island-setup-1.2.0.exe", &format!("{base}/v1.2.0/sky-island-setup-1.2.0.exe"))), "another spelling of the name");
        assert!(!offered(&attach("Sky-Island-Setup.exe", &format!("{base}/v1.2.0/Sky-Island-Setup.exe"))), "no version in the name");
    }

    #[test]
    fn the_checksum_of_a_known_text_comes_out_as_expected() {
        let mut hash = Sha256::new();
        hash.update(b"ab");
        hash.update(b"c");
        assert_eq!(hex_of(&hash.finalize()), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
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
        assert!(installer(&with(SETUP_URL, Some(DIGEST), 1234), "1.2.0").is_some());
        assert!(installer(&with(SETUP_URL, None, 1234), "1.2.0").is_none(), "no hash: not offered");
        assert!(installer(&with(SETUP_URL, Some("sha256:abc"), 1234), "1.2.0").is_none(), "not a SHA-256");
        assert!(installer(&with(SETUP_URL, Some(DIGEST), 0), "1.2.0").is_none(), "no size");
        assert!(installer(&with("https://example.com/Sky-Island-Setup-1.2.0.exe", Some(DIGEST), 1234), "1.2.0").is_none(), "another site");
        assert!(installer(&with("https://github.com/Other/Repo/releases/download/v1/Sky-Island-Setup-1.2.0.exe", Some(DIGEST), 1234), "1.2.0").is_none(), "another project");
    }

    fn available(version: &str) -> Status {
        Status::Available { version: version.to_string(), page: String::new(), asset: None }
    }

    #[test]
    fn a_check_by_itself_does_not_disturb_what_the_user_started() {
        let downloading = Status::Downloading { version: "1.2.0".into(), percent: 40 };
        assert_eq!(take_auto(&Status::Checking, available("1.2.0")), None);
        assert_eq!(take_auto(&downloading, available("1.3.0")), None);
        assert_eq!(take_auto(&downloading, Status::Latest), None);
    }

    #[test]
    fn a_check_by_itself_keeps_quiet_when_it_fails() {
        assert_eq!(take_auto(&Status::Idle, Status::Failed), None);
        assert_eq!(take_auto(&Status::Latest, Status::BadName), None);
        assert_eq!(take_auto(&available("1.2.0"), Status::Failed), None, "a known update stays known");
    }

    #[test]
    fn a_check_by_itself_takes_news_of_a_new_version() {
        assert_eq!(take_auto(&Status::Idle, available("1.2.0")), Some(available("1.2.0")));
        assert_eq!(take_auto(&Status::Latest, available("1.3.0")), Some(available("1.3.0")));
        assert_eq!(take_auto(&Status::Idle, Status::Latest), Some(Status::Latest));
    }

    #[test]
    fn a_release_page_from_somewhere_else_is_replaced_by_the_projects_own() {
        let odd = json!({ "tag_name": "v1.2.0", "html_url": "file:///C:/Windows/System32/calc.exe", "assets": [] });
        let Some(Status::Available { page, .. }) = judge(&odd, "1.1.0") else { panic!("an update") };
        assert!(page.starts_with("https://github.com/Dannyz15/Sky-Island/"));
    }
}
