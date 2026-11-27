//! The Microsoft Store build. The Store installs the app as an MSIX package, and then three things differ: the
//! Store updates the app (so the app does not look for updates itself), "Start with Windows" is a startup task
//! declared in the package instead of a Run value in the registry, and the data folder is private to the package.
//! The same exe runs both ways; it finds out here which one it is.

#[cfg(windows)]
mod win {
    use std::sync::OnceLock;

    use windows::ApplicationModel::{StartupTask, StartupTaskState};
    use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS};
    use windows::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;
    use windows::core::HSTRING;

    /// The id of the startup task in `AppxManifest.xml` (the `TaskId` of the `windows.startupTask` extension).
    const STARTUP_TASK: &str = "SkyIslandStartup";

    /// Whether this copy runs from an MSIX package, that is, was installed from the Microsoft Store.
    pub fn is_store() -> bool {
        static STORE: OnceLock<bool> = OnceLock::new();
        *STORE.get_or_init(|| {
            let mut length = 0u32;
            // SAFETY: a null buffer of length 0 only asks how long the package name is.
            let code = unsafe { GetCurrentPackageFullName(&mut length, None) };
            // A copy with no package gets APPMODEL_ERROR_NO_PACKAGE; a packaged one is told its name needs room.
            code == ERROR_INSUFFICIENT_BUFFER || code == ERROR_SUCCESS
        })
    }

    /// Whether the startup task is on.
    pub fn startup_enabled() -> bool {
        on_a_thread(|| Some(is_on(StartupTask::GetAsync(&HSTRING::from(STARTUP_TASK)).ok()?.join().ok()?.State().ok()?))).unwrap_or(false)
    }

    /// Turn the startup task on or off. When the user has turned it off in Task Manager or Settings, Windows does not let
    /// the app turn it on again; the switch in the app then stays off, because it shows what `startup_enabled` says.
    pub fn set_startup(on: bool) {
        on_a_thread(move || {
            let task = StartupTask::GetAsync(&HSTRING::from(STARTUP_TASK)).ok()?.join().ok()?;
            if on {
                task.RequestEnableAsync().ok()?.join().ok()?;
            } else {
                task.Disable().ok()?;
            }
            Some(())
        });
    }

    fn is_on(state: StartupTaskState) -> bool {
        state == StartupTaskState::Enabled || state == StartupTaskState::EnabledByPolicy
    }

    /// Run `work` on a thread of its own and wait for it: the Windows calls above wait for an answer, and the window's
    /// thread must not be the one that waits.
    fn on_a_thread<T: Send + 'static>(work: impl FnOnce() -> Option<T> + Send + 'static) -> Option<T> {
        std::thread::spawn(work).join().ok().flatten()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn only_an_enabled_task_counts_as_on() {
            assert!(is_on(StartupTaskState::Enabled));
            assert!(is_on(StartupTaskState::EnabledByPolicy));
            assert!(!is_on(StartupTaskState::Disabled));
            assert!(!is_on(StartupTaskState::DisabledByUser));
            assert!(!is_on(StartupTaskState::DisabledByPolicy));
        }
    }
}

#[cfg(windows)]
pub use win::{is_store, set_startup, startup_enabled};

/// Whether updates come from a package manager or app store rather than GitHub releases.
#[cfg(windows)]
pub fn updates_managed() -> bool {
    is_store()
}

#[cfg(not(windows))]
#[allow(dead_code)]
pub fn is_store() -> bool {
    false
}

#[cfg(any(target_os = "linux", test))]
fn is_linux_managed(exe: Option<&std::path::Path>, flatpak: bool, snap: bool, appimage: bool) -> bool {
    if appimage {
        return false;
    }
    if flatpak || snap {
        return true;
    }
    let Some(path) = exe else {
        return false;
    };
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    const SYSTEM_PREFIXES: [&str; 7] = ["/usr", "/bin", "/sbin", "/lib", "/lib64", "/opt", "/nix/store"];
    SYSTEM_PREFIXES.iter().any(|prefix| resolved.starts_with(prefix))
}

#[cfg(target_os = "linux")]
pub fn updates_managed() -> bool {
    use std::path::Path;
    use std::sync::OnceLock;

    static MANAGED: OnceLock<bool> = OnceLock::new();
    *MANAGED.get_or_init(|| {
        let flatpak = std::env::var_os("FLATPAK_ID").is_some_and(|v| !v.is_empty()) || Path::new("/.flatpak-info").exists();
        let snap = std::env::var_os("SNAP").is_some_and(|v| !v.is_empty());
        let appimage = std::env::var_os("APPIMAGE").is_some_and(|v| !v.is_empty());
        is_linux_managed(std::env::current_exe().ok().as_deref(), flatpak, snap, appimage)
    })
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn updates_managed() -> bool {
    false
}

#[cfg(not(windows))]
#[allow(dead_code)]
pub fn startup_enabled() -> bool {
    false
}

#[cfg(not(windows))]
#[allow(dead_code)]
pub fn set_startup(_on: bool) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn a_plain_exe_is_not_the_store_build() {
        assert!(!is_store(), "the tests do not run from a package");
        assert!(!updates_managed(), "cargo test builds are not package-managed");
    }

    #[test]
    fn linux_package_managed_detection_covers_system_paths_flatpak_snap_and_appimage() {
        for system_path in ["/usr/bin/sky-island", "/usr/local/bin/sky-island", "/opt/sky-island/sky-island", "/nix/store/abc-sky-island/bin/sky-island"] {
            assert!(is_linux_managed(Some(Path::new(system_path)), false, false, false), "{system_path}");
        }
        assert!(is_linux_managed(Some(Path::new("/home/user/sky-island")), true, false, false), "flatpak");
        assert!(is_linux_managed(Some(Path::new("/home/user/sky-island")), false, true, false), "snap");
        assert!(!is_linux_managed(Some(Path::new("/tmp/.mount_SkyIsl/usr/bin/sky-island")), false, false, true), "appimage stays self-updatable");
        assert!(!is_linux_managed(Some(Path::new("/home/user/Work/Sky-Island/target/release/sky-island")), false, false, false), "cargo build");
        assert!(!is_linux_managed(Some(Path::new("/home/user/.local/bin/sky-island")), false, false, false), "user bin");
        assert!(!is_linux_managed(None, false, false, false));
    }
}
