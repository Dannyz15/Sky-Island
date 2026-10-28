//! Writes `assets/icon.ico` from the same drawing code the tray icon uses. Each size is drawn
//! natively (not scaled down from one big picture), so the small ones stay crisp. Run with
//! `cargo run --example make_icon`; the result is embedded by build.rs.
//!
//! It also writes `assets/logo.png` (the logo for the README) and the two pictures shown inside the
//! installer (`installer/wizard-large.png` and `installer/wizard-small.png`). Pass a folder as the first argument to also write each icon size
//! as a PNG, for previewing.

#[path = "../src/appicon.rs"]
mod appicon;

use image::{ImageEncoder, codecs::png::PngEncoder};

const SIZES: [u32; 6] = [16, 32, 48, 64, 128, 256];

fn png(size: u32) -> Vec<u8> {
    let mut out = Vec::new();
    PngEncoder::new(&mut out)
        .write_image(&appicon::rgba(size), size, size, image::ExtendedColorType::Rgba8)
        .expect("encode png");
    out
}

fn main() {
    let images: Vec<(u32, Vec<u8>)> = SIZES.iter().map(|&s| (s, png(s))).collect();

    // ICO container: a 6-byte header, one 16-byte directory entry per image, then the PNG data.
    let mut ico = Vec::new();
    ico.extend_from_slice(&[0, 0, 1, 0]); // reserved, type = icon
    ico.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (size, data) in &images {
        ico.push(if *size >= 256 { 0 } else { *size as u8 }); // width, 0 means 256
        ico.push(if *size >= 256 { 0 } else { *size as u8 }); // height
        ico.extend_from_slice(&[0, 0]); // palette size, reserved
        ico.extend_from_slice(&1u16.to_le_bytes()); // colour planes
        ico.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        ico.extend_from_slice(&(data.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for (_, data) in &images {
        ico.extend_from_slice(data);
    }
    std::fs::write("assets/icon.ico", &ico).expect("write assets/icon.ico");
    println!("wrote assets/icon.ico ({} bytes, sizes {:?})", ico.len(), SIZES);

    // The logo as a PNG for the README, drawn large so it stays sharp when scaled down.
    std::fs::write("assets/logo.png", png(512)).expect("write assets/logo.png");
    println!("wrote assets/logo.png");

    // Installer wizard pictures, drawn at twice the size Inno Setup shows them so they stay sharp on
    // high-DPI screens (164x314 and 55x58 at 100%).
    appicon::wizard_banner(328, 628).save_png("installer/wizard-large.png").expect("write wizard-large.png");
    appicon::pixmap(110).save_png("installer/wizard-small.png").expect("write wizard-small.png");
    println!("wrote installer/wizard-large.png and installer/wizard-small.png");

    if let Some(dir) = std::env::args().nth(1) {
        for (size, data) in &images {
            std::fs::write(format!("{dir}/icon_{size}.png"), data).expect("write preview png");
        }
    }
}
