use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;

/// Hash every file under `dir` (path and contents).
fn hash_dir(dir: &Path, hasher: &mut DefaultHasher) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut paths: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            hash_dir(&path, hasher);
        } else if let Ok(bytes) = std::fs::read(&path) {
            path.hash(hasher);
            bytes.hash(hasher);
        }
    }
}

fn main() {
    // The frontend is embedded by tauri::generate_context!, which cargo does
    // not track. Feed a hash of ui/ into the crate's environment so any UI
    // change recompiles it and re-embeds the files.
    println!("cargo:rerun-if-changed=../ui");
    println!("cargo:rerun-if-changed=../gnome-extension");
    let mut hasher = DefaultHasher::new();
    hash_dir(Path::new("../ui"), &mut hasher);
    hash_dir(Path::new("../gnome-extension"), &mut hasher);
    println!("cargo:rustc-env=CLIPS_ASSETS_HASH={:x}", hasher.finish());

    tauri_build::build()
}
