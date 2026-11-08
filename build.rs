fn main() {
    // unrar_sys uses advapi32 (OpenProcessToken, CryptGenRandom) but does not link it.
    println!("cargo:rustc-link-lib=advapi32");
    println!("cargo:rerun-if-changed=assets/icon.ico");

    // Icon and the "Details" tab of the exe properties.
    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon("assets/icon.ico")
        .set("ProductName", "Deadlock Launcher Lite")
        .set("FileDescription", "Deadlock Launcher Lite")
        .set("OriginalFilename", "deadlock-launcher-lite.exe")
        .set("LegalCopyright", "MIT License");
    if let Err(e) = resource.compile() {
        // Building without the Windows SDK still works, just without an icon.
        println!("cargo:warning=exe icon not embedded: {e}");
    }
}
