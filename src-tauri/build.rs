fn main() {
    // The frontend in `frontendDist` is baked into this crate when it compiles,
    // but it is not one of its sources, so cargo cannot see that dependency: a
    // rebuilt `dist/` on its own leaves the previous bundle embedded in the
    // binary, and the app goes on serving a UI that is no longer on disk.
    println!("cargo:rerun-if-changed=../flow-insights-dashboard/dist");
    tauri_build::build()
}
