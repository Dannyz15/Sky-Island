fn main() {
    // Give the exe its icon (and a version resource) so Explorer and the installer show it.
    let _ = embed_resource::compile("assets/app.rc", embed_resource::NONE);
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");
}
