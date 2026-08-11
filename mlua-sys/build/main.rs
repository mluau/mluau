use std::env;

#[path = "find_vendored.rs"]
mod find;

fn main() {
    println!("cargo:rerun-if-changed=build");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    if target_os == "windows" && cfg!(feature = "module") {
        if !std::env::var("LUA_LIB_NAME").unwrap_or_default().is_empty() {
            // Don't use raw-dylib linking
            find::probe_lua();
            return;
        }

        println!("cargo:rustc-cfg=raw_dylib");
    }

    find::probe_lua();
}
