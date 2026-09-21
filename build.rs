use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=res/clipcywin.rc");
    println!("cargo:rerun-if-changed=res/clipcywin.manifest");
    println!("cargo:rerun-if-changed=res/clipcywin.ico");
    println!("cargo:rerun-if-changed=res/WebView2Loader.dll");
    embed_resource::compile("res/clipcywin.rc", embed_resource::NONE).manifest_required().unwrap();

    // WebView2Loader is linked as raw-dylib; ship the DLL next to the executable.
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    // OUT_DIR = target/<profile>/build/<pkg>-<hash>/out → profile dir is 3 levels up
    if let Some(profile_dir) = out_dir.ancestors().nth(3) {
        let dst = profile_dir.join("WebView2Loader.dll");
        let _ = std::fs::copy("res/WebView2Loader.dll", dst);
    }
}
