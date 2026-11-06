fn main() {
    println!("cargo:rerun-if-changed=updater.pub");
    let windows_msvc = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    // Tauri's default manifest contains the same Common Controls dependency,
    // but its resource linker argument applies only to binaries, not lib tests.
    // Embed it through MSVC for every executable instead. Keep icon/version
    // resources, and disable only their manifest to avoid duplicate resource 1.
    let attributes = if windows_msvc {
        tauri_build::Attributes::new()
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest())
    } else {
        tauri_build::Attributes::new()
    };
    tauri_build::try_build(attributes).expect("Could not build desktop resources");
    if windows_msvc {
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'"
        );
    }
}
