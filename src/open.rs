//! Opening a file, a folder or a web address the way a double-click does.
//!
//! This used to start `explorer.exe` with the path as its argument. Explorer reads its own command line as a list
//! separated by commas (`/select,<path>`), so `C:\Downloads\setup,notes.pdf` opened `C:\Downloads\setup` instead:
//! another item than the one shown on the shelf. Asking the shell to open the exact string has no such reading.
//! It also never looks for a program named `explorer` next to the app first.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::thread;

use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize};
use windows::Win32::UI::Shell::{SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::PCWSTR;

/// Open `target` (a file, a folder or an `https://` address) with whatever Windows opens it with: exactly that
/// item, and nothing in it is read as a switch. It runs on its own thread, because the shell may wait on a slow or
/// unreachable network drive, and the notch must not wait with it.
pub fn open(target: impl AsRef<OsStr>) {
    let wide: Vec<u16> = target.as_ref().encode_wide().chain(Some(0)).collect();
    let _ = thread::Builder::new().name("open".into()).spawn(move || shell_open(&wide));
}

/// `wide` is the NUL-ended name to open.
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
}
