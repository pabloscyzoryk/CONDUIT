//! Resolve the packaged interpreter without depending on the launch directory.
use std::path::{Path, PathBuf};

pub(crate) fn resolve_python(configured: Option<&str>, executable_dir: &Path) -> PathBuf {
    let configured = configured.unwrap_or("").trim();
    let default_command = configured.is_empty()
        || configured.eq_ignore_ascii_case("python")
        || configured.eq_ignore_ascii_case("python.exe");
    if default_command {
        let bundled = executable_dir.join("runtime").join("python.exe");
        return if bundled.is_file() {
            bundled
        } else {
            PathBuf::from("python")
        };
    }
    let explicit = PathBuf::from(configured);
    if explicit.is_absolute() {
        explicit
    } else if configured.contains('/') || configured.contains('\\') {
        executable_dir.join(explicit)
    } else {
        // An explicit command such as python3 or py keeps the user's PATH choice.
        explicit
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "conduit_python_resolver_{}_{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(root.join("runtime")).unwrap();
            Self(root)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(self.0.join("runtime/python.exe"));
            let _ = std::fs::remove_dir(self.0.join("runtime"));
            let _ = std::fs::remove_dir(&self.0);
        }
    }

    #[test]
    fn portable_interpreter_is_used_for_default_choices() {
        let fixture = Fixture::new();
        let executable = fixture.0.join("runtime/python.exe");
        std::fs::write(&executable, b"synthetic marker, never executed").unwrap();
        for value in [None, Some(""), Some(" python "), Some("PYTHON.EXE")] {
            assert_eq!(resolve_python(value, &fixture.0), executable);
        }
    }

    #[test]
    fn explicit_custom_interpreter_is_never_silently_replaced() {
        let fixture = Fixture::new();
        std::fs::write(fixture.0.join("runtime/python.exe"), b"marker").unwrap();
        let absolute = fixture.0.join("custom/python.exe");
        assert_eq!(resolve_python(absolute.to_str(), &fixture.0), absolute);
        assert_eq!(
            resolve_python(Some("python3"), &fixture.0),
            PathBuf::from("python3")
        );
        assert_eq!(
            resolve_python(Some("custom/python.exe"), &fixture.0),
            absolute
        );
    }

    #[test]
    fn development_without_bundle_keeps_path_fallback() {
        let fixture = Fixture::new();
        assert_eq!(resolve_python(None, &fixture.0), PathBuf::from("python"));
        assert_eq!(
            resolve_python(Some("python.exe"), &fixture.0),
            PathBuf::from("python")
        );
    }
}
