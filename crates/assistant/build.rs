//! Optional open fonts use the same build input as the other Craft apps.
fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    if let Err(error) = craft_fonts_build::Embed::new().write("craft_fonts.rs") {
        println!("cargo::error={error}");
    }
}
