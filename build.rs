use std::{env, fs, path::PathBuf};

/// The Windows resources of the exe: its icon, and its version information (what Explorer shows under
/// Properties, Details, and what Task Manager calls the app). The version is taken from Cargo.toml, so it
/// cannot differ from the version the app and the installer report.
fn resource_script(version: &str, numbers: [u32; 4], icon: &str) -> String {
    let [a, b, c, d] = numbers;
    format!(
        r#"1 ICON "{icon}"

1 VERSIONINFO
FILEVERSION {a},{b},{c},{d}
PRODUCTVERSION {a},{b},{c},{d}
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904b0"
        BEGIN
            VALUE "CompanyName", "Dannyz15"
            VALUE "FileDescription", "Sky Island"
            VALUE "FileVersion", "{version}"
            VALUE "InternalName", "sky-island"
            VALUE "LegalCopyright", "Copyright (c) 2026 Dannyz15. MIT License."
            VALUE "OriginalFilename", "sky-island.exe"
            VALUE "ProductName", "Sky Island"
            VALUE "ProductVersion", "{version}"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x409, 1200
    END
END
"#
    )
}

fn main() {
    let number = |name: &str| env::var(name).ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
    let numbers = [
        number("CARGO_PKG_VERSION_MAJOR"),
        number("CARGO_PKG_VERSION_MINOR"),
        number("CARGO_PKG_VERSION_PATCH"),
        0,
    ];
    let version = env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let manifest = env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    // The resource compiler reads backslashes in a string as escapes, so the path uses forward slashes.
    let icon = format!("{}/assets/icon.ico", manifest.replace('\\', "/"));

    let script = PathBuf::from(env::var("OUT_DIR").unwrap_or_default()).join("app.rc");
    if fs::write(&script, resource_script(&version, numbers, &icon)).is_ok() {
        let _ = embed_resource::compile(&script, embed_resource::NONE);
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=Cargo.toml");
}
