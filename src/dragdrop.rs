//! Start an OS drag of a file out of the notch, using the shell's own data object so the
//! drop target (Explorer, a browser, a chat app...) sees a normal file drag.

use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;

use windows::Win32::Foundation::{DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, S_OK};
use windows::Win32::System::Com::{CoTaskMemFree, IDataObject};
use windows::Win32::System::Ole::{
    DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, DoDragDrop, IDropSource, IDropSource_Impl,
};
use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    BHID_DataObject, IShellItem, SHCreateItemFromParsingName, SHCreateShellItemArrayFromIDLists, SHParseDisplayName,
};
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
