// Compiles the firmware's ADPCM encoder so tests can check it against the Rust decoder.
fn main() {
    println!("cargo:rerun-if-changed=../firmware/src/adpcm.c");
    cc::Build::new().file("../firmware/src/adpcm.c").compile("stv_adpcm_c");
}
