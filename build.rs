use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=assets");
    println!("cargo:rerun-if-changed=templates");

    let mut hasher = DefaultHasher::new();
    if hash_tree(Path::new("assets"), &mut hasher).is_err() {
        println!("cargo:warning=failed to hash assets, cache busting falls back to package version");
    }
    if hash_tree(Path::new("templates"), &mut hasher).is_err() {
        println!("cargo:warning=failed to hash templates, cache busting falls back to package version");
    }

    let pkg = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string());
    println!("cargo:rustc-env=ONEPAGE_ASSET_VERSION={pkg}-{:016x}", hasher.finish());
}

fn hash_tree(path: &Path, hasher: &mut DefaultHasher) -> std::io::Result<()> {
    let mut entries = fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        path.hash(hasher);
        if path.is_dir() {
            hash_tree(&path, hasher)?;
        } else {
            fs::read(&path)?.hash(hasher);
        }
    }
    Ok(())
}
