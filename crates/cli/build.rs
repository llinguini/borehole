// Resolves the version reported by the binary at runtime.
//
// CI sets `BOREHOLE_VERSION` to the release tag (e.g. `v0.1.0`) so the built
// artifact always matches the published Release. When the variable is absent
// (local/dev builds) we fall back to the crate version from `Cargo.toml`.
fn main() {
    let version = std::env::var("BOREHOLE_VERSION")
        .ok()
        .map(|v| v.trim().trim_start_matches('v').to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| std::env::var("CARGO_PKG_VERSION").unwrap_or_default());

    println!("cargo:rustc-env=BOREHOLE_VERSION={version}");
    println!("cargo:rerun-if-env-changed=BOREHOLE_VERSION");
}
