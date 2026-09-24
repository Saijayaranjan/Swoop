//! Rebuild when the embedded web UI bundle changes (rust-embed only tracks files that existed at
//! the previous build, so a fresh `npm run build` producing new hashed names would be missed).

fn main() {
    println!("cargo:rerun-if-changed=../../web/remote/dist");
    println!("cargo:rerun-if-changed=build.rs");
}
