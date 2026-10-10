#[path = "../src/font.rs"]
mod font;
fn main() {
    println!("cargo:rerun-if-env-changed=WIFI_SSID");
    println!("cargo:rerun-if-env-changed=WIFI_PASSWORD");
    println!("cargo:rerun-if-changed=../japanese_font.bin");
    println!("cargo:rerun-if-changed=../src/font.rs");
    let raw = std::fs::read("../japanese_font.bin").expect("read font atlas");
    let packed = font::pack(&raw);
    println!(
        "cargo:warning=Japanese font: {} -> {} bytes",
        raw.len(),
        packed.len()
    );
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(out.join("japanese_font.jpf"), packed).expect("write packed font");
    embuild::espidf::sysenv::output();
}
