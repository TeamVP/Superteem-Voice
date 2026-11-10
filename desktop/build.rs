fn main() {
    // sherpa-onnx (the natural reading voice) is loaded only when it's first used, so the app starts on a
    // PC where its DLL is missing; `tts` checks it's there before calling into it.
    let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    if windows && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg=/DELAYLOAD:sherpa-onnx-c-api.dll");
        println!("cargo:rustc-link-arg=delayimp.lib");
    }
    tauri_build::build()
}
