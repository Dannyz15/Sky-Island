//! Opening a file, a folder or a web address the way a double-click does.
//!
//! This used to start `explorer.exe` with the path as its argument. Explorer reads its own command line as a list
//! separated by commas (`/select,<path>`), so `C:\Downloads\setup,notes.pdf` opened `C:\Downloads\setup` instead:
//! another item than the one shown on the shelf. Asking the shell to open the exact string has no such reading.
//! It also never looks for a program named `explorer` next to the app first.

use std::ffi::OsStr;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(any(windows, target_os = "linux"))]
use std::thread;

#[cfg(windows)]
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize};
#[cfg(windows)]
use windows::Win32::UI::Shell::{SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
#[cfg(windows)]
use windows::core::PCWSTR;

/// Open `target` (a file, a folder or an `https://` address) with whatever Windows opens it with: exactly that
/// item, and nothing in it is read as a switch. It runs on its own thread, because the shell may wait on a slow or
/// unreachable network drive, and the notch must not wait with it.
pub fn open(target: impl AsRef<OsStr>) {
    #[cfg(windows)]
    {
        let wide: Vec<u16> = target.as_ref().encode_wide().chain(Some(0)).collect();
        let _ = thread::Builder::new().name("open".into()).spawn(move || shell_open(&wide));
    }
    #[cfg(target_os = "linux")]
    if let Some(arg) = linux::prepare_target(target.as_ref()) {
        let _ = thread::Builder::new().name("open".into()).spawn(move || linux::open_target(&arg));
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    let _ = target;
}

#[cfg(target_os = "linux")]
mod linux {
    use std::ffi::{OsStr, OsString};
    use std::io;
    use std::os::unix::ffi::OsStrExt;
    use std::process::{Command, Stdio};

    pub(super) fn prepare_target(target: &OsStr) -> Option<OsString> {
        let bytes = target.as_bytes();
        if bytes.is_empty() || bytes.contains(&0) {
            return None;
        }
        if let Some(text) = target.to_str() {
            if text.chars().any(char::is_control) {
                return None;
            }
            if text.starts_with("https://") || text.starts_with("http://") {
                return Some(target.to_os_string());
            }
        }
        if bytes.starts_with(b"-") {
            let resolved = std::env::current_dir().map(|dir| dir.join(target).into_os_string()).unwrap_or_else(|_| {
                let mut out = OsString::from("./");
                out.push(target);
                out
            });
            return Some(resolved);
        }
        Some(target.to_os_string())
    }

    pub(super) fn open_target(target: &OsStr) {
        let run = |program: &str, args: &[&OsStr]| -> Option<()> {
            match Command::new(program).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
                Ok(mut child) => {
                    let _ = child.wait();
                    Some(())
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => None,
                Err(_) => Some(()),
            }
        };
        if run("xdg-open", &[target]).is_none() {
            let _ = run("gio", &[OsStr::new("open"), OsStr::new("--"), target]);
        }
    }
}

/// `wide` is the NUL-ended name to open.
#[cfg(windows)]
fn shell_open(wide: &[u16]) {
    unsafe {
        // The shell wants COM on the thread that calls it.
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        let mut info = SHELLEXECUTEINFOW {
            cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
            // No verb: the default one, the same as a double-click. NOASYNC: finish the work before this thread ends.
            fMask: SEE_MASK_NOASYNC,
            lpFile: PCWSTR(wide.as_ptr()),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        let _ = ShellExecuteExW(&mut info);
        if com.is_ok() {
            CoUninitialize();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run by hand (`cargo test opening_a_path -- --ignored`): it opens a real Explorer window, so it is not part of
    /// the normal run. Whoever runs it looks at which folder the window shows: it must be `x,y`, not `x`.
    #[test]
    #[ignore = "opens a window"]
    fn opening_a_path_with_a_comma_opens_that_exact_folder() {
        let base = std::env::temp_dir().join("sky-open-test");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("x,y")).unwrap();
        std::fs::create_dir_all(base.join("x")).unwrap();
        open(base.join("x,y"));
        std::thread::sleep(std::time::Duration::from_secs(12));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_prepare_target_keeps_urls_commas_and_guards_dashes() {
        assert_eq!(linux::prepare_target(OsStr::new("https://github.com/Dannyz15/Sky-Island")).as_deref(), Some(OsStr::new("https://github.com/Dannyz15/Sky-Island")));
        assert_eq!(linux::prepare_target(OsStr::new("/tmp/x,y/my notes.pdf")).as_deref(), Some(OsStr::new("/tmp/x,y/my notes.pdf")));

        for weird in ["-weird.txt", "--help"] {
            let prepared = linux::prepare_target(OsStr::new(weird)).expect("dash-prefixed path stays valid");
            assert!(!prepared.to_string_lossy().starts_with('-'), "should not start with dash: {prepared:?}");
            assert!(prepared.to_string_lossy().ends_with(weird));
        }

        assert_eq!(linux::prepare_target(OsStr::new("")), None);
        assert_eq!(linux::prepare_target(OsStr::new("bad\nline")), None);
        assert_eq!(linux::prepare_target(OsStr::new("bad\0nul")), None);
    }
}
