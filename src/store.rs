//! The app's small JSON files in %APPDATA%\Sky-Island, read and written so that one bad moment (a crash,
//! a power cut, a file damaged by hand) cannot cost the user their data.

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use serde_json::Value;

/// A file in the app's data folder, which is created when it is missing.
pub fn path(name: &str) -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("APPDATA")?).join("Sky-Island");
    fs::create_dir_all(&dir).ok()?;
    Some(dir.join(name))
}

/// The JSON in a file. `None` when there is no file. A file that cannot be read or understood is not
/// left in place, where the next save would overwrite it: it is renamed to `<name>.corrupt` (the
/// previous copy of that is replaced), so the user can still get to what was in it.
pub fn read_json(path: &Path) -> Option<Value> {
    let parsed = match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str::<Value>(&text).ok(),
        Err(e) if e.kind() == ErrorKind::NotFound => return None,
        Err(_) => None,
    };
    if parsed.is_none() {
        let mut kept = path.as_os_str().to_owned();
        kept.push(".corrupt");
        let _ = fs::rename(path, kept);
    }
    parsed
}

/// Replace a file with `text` all at once: the text is written to a temporary file next to it and then
/// renamed over it, so the file is always either the old version or the new one, never half of one.
pub fn write(path: &Path, text: &str) {
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    let done = fs::File::create(&temp)
        .and_then(|mut f| f.write_all(text.as_bytes()).and_then(|()| f.sync_all()))
        .and_then(|()| fs::rename(&temp, path));
    if done.is_err() {
        let _ = fs::remove_file(&temp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sky-island-store-test-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_write_replaces_the_file_and_leaves_no_temporary_one() {
        let dir = scratch("write");
        let file = dir.join("a.json");
        write(&file, "[1]");
        write(&file, "[1,2]");
        assert_eq!(fs::read_to_string(&file).unwrap(), "[1,2]");
        assert!(!dir.join("a.json.tmp").exists());
        assert_eq!(read_json(&file).unwrap().as_array().unwrap().len(), 2);
    }

    #[test]
    fn a_missing_file_is_just_nothing() {
        let dir = scratch("missing");
        assert!(read_json(&dir.join("none.json")).is_none());
        assert!(fs::read_dir(&dir).unwrap().next().is_none(), "nothing is created or renamed");
    }

    #[test]
    fn a_damaged_file_is_kept_aside_instead_of_being_overwritten() {
        let dir = scratch("damaged");
        let file = dir.join("todo.json");
        fs::write(&file, "[{\"text\": \"buy milk\"").unwrap();
        assert!(read_json(&file).is_none());
        assert!(!file.exists(), "the next save writes a fresh file");
        assert_eq!(fs::read_to_string(dir.join("todo.json.corrupt")).unwrap(), "[{\"text\": \"buy milk\"", "what was in it is still there");
    }
}
