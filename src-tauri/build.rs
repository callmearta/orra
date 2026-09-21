fn main() {
    // The frontend in `frontendDist` is baked into this crate when it compiles,
    // but it is not one of its sources, so cargo cannot see that dependency: a
    // rebuilt `dist/` on its own leaves the previous bundle embedded in the
    // binary, and the app goes on serving a UI that is no longer on disk.
    println!("cargo:rerun-if-changed=../ui/dist");
    // The macOS engine hash is baked into the binary by `engine.rs`; changing it
    // in the environment has to rebuild, or a release would keep the previous
    // download's checksum.
    println!("cargo:rerun-if-env-changed=ORRA_MACOS_ENGINE_SHA256");
    tauri_build::build()
}
