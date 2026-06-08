// Build script: optionally builds the SvelteKit dashboard so it can be embedded
// in the binary via `include_dir!`.
//
// The dashboard is rebuilt only on a release build or when `WEB_BUILD=1` is set,
// so day-to-day `cargo build` stays fast and needs no Node toolchain. The
// embedded directory (`web/build`) is always ensured to exist, because
// `include_dir!` fails to compile against a missing path: this lets the server
// compile (and serve a 404 at `/`) even when the dashboard was never built.

use std::path::Path;

fn main() {
    // Rebuild when the dashboard sources change (release / WEB_BUILD only act
    // below, but this keeps the embedded assets fresh when they do run).
    println!("cargo:rerun-if-changed=../../web/src");
    println!("cargo:rerun-if-env-changed=WEB_BUILD");

    let build_dir = Path::new("../../web/build");

    let should_build = std::env::var("WEB_BUILD").is_ok()
        || std::env::var("PROFILE").unwrap_or_default() == "release";

    if should_build {
        let status = std::process::Command::new("npm")
            .args(["run", "build"])
            .current_dir("../../web")
            .status()
            .expect("npm not found — install Node.js to build the dashboard");
        assert!(status.success(), "npm run build failed");
    }

    // Guarantee the embedded directory exists so `include_dir!` always compiles,
    // even on a fresh checkout where the dashboard has never been built.
    if !build_dir.exists() {
        std::fs::create_dir_all(build_dir).expect("failed to create web/build placeholder");
    }
}
