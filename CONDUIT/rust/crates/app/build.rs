fn main() {
    // Pin the public production Rust source used by the offline replay driver.
    // Only hashes are embedded; no runtime configuration or credentials read.
    use sha2::{Digest, Sha256};
    use std::path::Path;
    fn collect(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        for item in std::fs::read_dir(dir).expect("public source directory") {
            let path = item.expect("source entry").path();
            if path.is_dir() {
                collect(&path, out);
            } else if path.extension().and_then(|v| v.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for item in std::fs::read_dir(root.join("crates")).expect("crates") {
        let dir = item.expect("crate entry").path();
        if dir.join("src").is_dir() {
            collect(&dir.join("src"), &mut files);
        }
        if dir.join("Cargo.toml").is_file() {
            files.push(dir.join("Cargo.toml"));
        }
        if dir.join("build.rs").is_file() {
            files.push(dir.join("build.rs"));
        }
    }
    files.push(root.join("Cargo.toml"));
    files.push(root.join("Cargo.lock"));
    files.sort();
    let mut hash = Sha256::new();
    for file in files {
        println!("cargo:rerun-if-changed={}", file.display());
        let relative = file
            .strip_prefix(&root)
            .expect("source under workspace")
            .to_string_lossy()
            .replace('\\', "/");
        hash.update(relative.as_bytes());
        hash.update([0]);
        hash.update(std::fs::read(file).expect("source bytes"));
        hash.update([0]);
    }
    println!(
        "cargo:rustc-env=CONDUIT_REPLAY_SOURCE_SHA256={:x}",
        hash.finalize()
    );
    // Tauri generuje kontekst (konfiguracja, uprawnienia, zasoby Windows).
    // Bez cechy `window` binarka jest czysto serwerowa i nic tu nie robimy.
    #[cfg(feature = "window")]
    tauri_build::build();
}
