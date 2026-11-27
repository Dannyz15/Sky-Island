//! Start an OS drag of a file out of the notch, using the shell's own data object so the
//! drop target (Explorer, a browser, a chat app...) sees a normal file drag.

#[cfg(windows)]
mod win {
    use std::os::windows::ffi::OsStrExt;
    use std::path::PathBuf;

    use windows::Win32::Foundation::{DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, S_OK};
    use windows::Win32::System::Com::{CoTaskMemFree, IDataObject};
    use windows::Win32::System::Ole::{DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, DoDragDrop, IDropSource, IDropSource_Impl};
    use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
    use windows::Win32::UI::Shell::Common::ITEMIDLIST;
    use windows::Win32::UI::Shell::{BHID_DataObject, IShellItem, SHCreateItemFromParsingName, SHCreateShellItemArrayFromIDLists, SHParseDisplayName};
    use windows::core::{BOOL, HRESULT, PCWSTR, Result, implement};

    #[implement(IDropSource)]
    struct DropSource;

    impl IDropSource_Impl for DropSource_Impl {
        fn QueryContinueDrag(&self, escape_pressed: BOOL, key_state: MODIFIERKEYS_FLAGS) -> HRESULT {
            if escape_pressed.as_bool() {
                DRAGDROP_S_CANCEL
            } else if !key_state.contains(MK_LBUTTON) {
                DRAGDROP_S_DROP
            } else {
                S_OK
            }
        }

        fn GiveFeedback(&self, _effect: DROPEFFECT) -> HRESULT {
            DRAGDROP_S_USEDEFAULTCURSORS
        }
    }

    /// Run a drag of one or more files until the user drops or cancels. Blocks while the drag is in
    /// progress. Returns whether they were dropped on something that took them (a copy, a move or a
    /// link), as opposed to being cancelled or let go over nothing.
    pub fn drag_files(paths: &[PathBuf]) -> Result<bool> {
        unsafe {
            let data = data_object(paths)?;
            let source: IDropSource = DropSource.into();
            let mut effect = DROPEFFECT(0);
            // DoDragDrop reports drop/cancel through its HRESULT; neither is a failure here.
            let result = DoDragDrop(&data, &source, DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK, &mut effect);
            Ok(result == DRAGDROP_S_DROP && effect.0 != 0)
        }
    }

    /// The shell's own data object for the files, which carries them in every format a target may ask for.
    fn data_object(paths: &[PathBuf]) -> Result<IDataObject> {
        unsafe {
            if let [path] = paths {
                let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
                let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None)?;
                return item.BindToHandler(None, &BHID_DataObject);
            }
            let mut ids = Vec::new();
            for path in paths {
                let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
                let mut id = std::ptr::null_mut();
                if SHParseDisplayName(PCWSTR(wide.as_ptr()), None, &mut id, 0, None).is_ok() {
                    ids.push(id as *const ITEMIDLIST);
                }
            }
            let array = SHCreateShellItemArrayFromIDLists(&ids);
            for id in &ids {
                CoTaskMemFree(Some(*id as *const _));
            }
            array?.BindToHandler(None, &BHID_DataObject)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};

        #[test]
        fn several_files_travel_as_a_normal_file_drop() {
            let dir = std::env::temp_dir().join("sky-island-drag-test");
            std::fs::create_dir_all(&dir).unwrap();
            let paths: Vec<PathBuf> = ["a.txt", "b.txt"].iter().map(|n| dir.join(n)).collect();
            for p in &paths {
                std::fs::write(p, "x").unwrap();
            }
            unsafe {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                let data = data_object(&paths).unwrap();
                let hdrop = FORMATETC { cfFormat: 15, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 };
                assert!(data.QueryGetData(&hdrop).is_ok(), "a file-drop (CF_HDROP) format is offered");
            }
        }
    }
}

#[cfg(windows)]
pub use win::drag_files;

#[cfg(not(windows))]
pub fn drag_files(_paths: &[std::path::PathBuf]) -> std::result::Result<bool, ()> {
    Ok(false)
}

#[cfg(any(target_os = "linux", test))]
fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(any(target_os = "linux", test))]
fn percent_decode(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex_nibble(bytes[i + 1]), hex_nibble(bytes[i + 2]))
        {
            out.push((hi << 4) | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

/// Parse an RFC 2483 `text/uri-list` payload into local filesystem paths (ignoring comments, non-`file://` URIs, and remote hosts).
#[cfg(any(target_os = "linux", test))]
pub fn parse_uri_list(raw: &str) -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    for line in raw.lines() {
        let line = line.trim_end_matches('\r').trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(rest) = line.strip_prefix("file://localhost").or_else(|| line.strip_prefix("file://")) else {
            continue;
        };
        if !rest.starts_with('/') {
            continue;
        }
        let decoded = percent_decode(rest);
        #[cfg(unix)]
        let path = {
            use std::ffi::OsString;
            use std::os::unix::ffi::OsStringExt;
            std::path::PathBuf::from(OsString::from_vec(decoded))
        };
        #[cfg(not(unix))]
        let path = std::path::PathBuf::from(String::from_utf8_lossy(&decoded).into_owned());
        paths.push(path);
    }
    paths
}

/// Encode local filesystem paths as an RFC 2483 `text/uri-list` payload (`\r\n`-delimited `file://` URIs).
#[cfg(any(target_os = "linux", test))]
pub fn encode_uri_list(paths: &[std::path::PathBuf]) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for path in paths {
        #[cfg(unix)]
        let raw = {
            use std::os::unix::ffi::OsStrExt;
            path.as_os_str().as_bytes()
        };
        #[cfg(not(unix))]
        let raw_owned = path.to_string_lossy().into_owned();
        #[cfg(not(unix))]
        let raw = raw_owned.as_bytes();

        out.push_str("file://");
        for &b in raw {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/') {
                out.push(b as char);
            } else {
                let _ = write!(out, "%{b:02X}");
            }
        }
        out.push_str("\r\n");
    }
    out
}

#[cfg(test)]
mod uri_tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn uri_list_parses_local_files_and_skips_comments_and_remote_hosts() {
        let raw = "# comment\r\n\r\nhttps://example.com/a.txt\r\nfile://remote-host/share/a.txt\r\nfile:///tmp/hello%20world%231.txt\r\nfile://localhost/tmp/plain.txt\r\n";
        let got = parse_uri_list(raw);
        assert_eq!(got, vec![PathBuf::from("/tmp/hello world#1.txt"), PathBuf::from("/tmp/plain.txt")]);
    }

    #[test]
    fn uri_list_round_trips_special_characters_and_utf8() {
        let paths = vec![PathBuf::from("/tmp/My File (100% #ok?).txt"), PathBuf::from("/tmp/notes-日本語.md")];
        let encoded = encode_uri_list(&paths);
        assert!(encoded.starts_with("file:///tmp/My%20File%20%28100%25%20%23ok%3F%29.txt\r\n"));
        assert_eq!(parse_uri_list(&encoded), paths);
    }
}
