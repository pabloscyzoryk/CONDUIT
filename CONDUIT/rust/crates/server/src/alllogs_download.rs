//! Raw-byte download of the last completed allLogs export, not a file browser.
use crate::state::StateHandle;
use axum::body::Body;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

// Replay frames already enforce 1 GiB including preceding history text.
// Source inventory/footer is appended afterwards, so reserve explicit headroom.
const MAX_BYTES: u64 = 1024 * 1024 * 1024 + 4 * 1024 * 1024;
const CHUNK_BYTES: usize = 64 * 1024;

fn fail(status: StatusCode, code: &str) -> Response {
    (status, Json(json!({"ok":false,"error":code}))).into_response()
}

fn no_reparse(path: &Path) -> std::io::Result<PathBuf> {
    if path
        .components()
        .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(std::io::Error::other("parent path component refused"));
    }
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut prefix = PathBuf::new();
    for component in absolute.components() {
        prefix.push(component.as_os_str());
        if matches!(component, Component::Prefix(_) | Component::RootDir) {
            continue;
        }
        let metadata = std::fs::symlink_metadata(&prefix)?;
        #[cfg(windows)]
        let reparse = {
            use std::os::windows::fs::MetadataExt;
            metadata.file_attributes() & 0x400 != 0
        };
        #[cfg(not(windows))]
        let reparse = false;
        if reparse || metadata.file_type().is_symlink() {
            return Err(std::io::Error::other("reparse path refused"));
        }
    }
    absolute.canonicalize()
}

fn open_completed(
    st: &StateHandle,
    requested: &str,
) -> Result<(std::fs::File, u64, String), &'static str> {
    let completed = st
        .read(|s| {
            (!s.scalanie.aktywne && s.scalanie.faza == "gotowe").then(|| s.scalanie.sciezka.clone())
        })
        .filter(|s| !s.is_empty())
        .ok_or("No completed allLogs export is available")?;
    let actual = no_reparse(Path::new(&completed)).map_err(|_| "Unsafe allLogs export path")?;
    let asked = no_reparse(Path::new(requested)).map_err(|_| "Unsafe allLogs download path")?;
    if asked != actual {
        return Err("Only the completed allLogs export can be downloaded");
    }
    let root = crate::alllogs::katalog_docelowy(st).unwrap_or_else(|| st.workspace.logs_dir());
    let root = no_reparse(&root).map_err(|_| "Unsafe allLogs output directory")?;
    if actual.parent() != Some(root.as_path()) {
        return Err("allLogs export is outside its output directory");
    }
    let name = actual
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("Invalid allLogs export name")?;
    if !name.starts_with("alllogs_")
        || !name.ends_with(".txt")
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
    {
        return Err("Invalid allLogs export name");
    }
    let file =
        std::fs::File::open(&actual).map_err(|_| "Cannot open the completed allLogs export")?;
    let metadata = file
        .metadata()
        .map_err(|_| "Cannot inspect the completed allLogs export")?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return Err("allLogs download byte limit exceeded");
    }
    Ok((file, metadata.len(), name.to_owned()))
}

pub(super) async fn download(State(st): State<StateHandle>, Json(body): Json<Value>) -> Response {
    let Some(requested) = body["path"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
    else {
        return fail(StatusCode::BAD_REQUEST, "Missing allLogs export path");
    };
    let opened = tokio::task::spawn_blocking(move || open_completed(&st, &requested)).await;
    let (file, size, name) = match opened {
        Ok(Ok(file)) => file,
        Ok(Err(code)) => return fail(StatusCode::FORBIDDEN, code),
        Err(_) => {
            return fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "allLogs download worker failed",
            )
        }
    };
    let stream =
        futures_util::stream::try_unfold((file, size), |(mut file, remaining)| async move {
            if remaining == 0 {
                return Ok::<_, std::io::Error>(None);
            }
            tokio::task::spawn_blocking(move || {
                let mut bytes = vec![0; CHUNK_BYTES.min(remaining as usize)];
                file.read_exact(&mut bytes)?;
                let remaining = remaining - bytes.len() as u64;
                Ok(Some((bytes, (file, remaining))))
            })
            .await
            .map_err(std::io::Error::other)?
        });
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{name}\""),
        )
        .header(header::CONTENT_LENGTH, size.to_string())
        .header(header::CACHE_CONTROL, "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .body(Body::from_stream(stream))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state(label: &str) -> StateHandle {
        let dir = std::env::temp_dir().join(format!(
            "conduit-history-download-{}-{}-{label}",
            std::process::id(),
            crate::now_ms()
        ));
        std::fs::create_dir_all(dir.join("logs")).unwrap();
        crate::bootstrap(
            &crate::ServerConfig {
                workspace: dir,
                ..Default::default()
            },
            crate::default_auth(),
        )
        .unwrap()
    }
    fn completed(st: &StateHandle, path: &Path) {
        st.update_transient(
            crate::coalesce::Sections::one(crate::coalesce::Section::Scalanie),
            |s| {
                s.scalanie.faza = "gotowe".into();
                s.scalanie.sciezka = path.display().to_string();
            },
        );
    }
    #[tokio::test]
    async fn actual_stream_over_32mib_preserves_every_raw_byte() {
        let st = state("large");
        let path = st.workspace.logs_dir().join("alllogs_synthetic.txt");
        let mut bytes = vec![0u8; 33 * 1024 * 1024 + 17];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        std::fs::write(&path, &bytes).unwrap();
        completed(&st, &path);
        let response = download(State(st.clone()), Json(json!({"path":path}))).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_LENGTH],
            bytes.len().to_string()
        );
        let actual = axum::body::to_bytes(response.into_body(), bytes.len() + 1)
            .await
            .unwrap();
        assert_eq!(actual.as_ref(), bytes.as_slice());
        std::fs::remove_dir_all(&st.workspace.root).unwrap();
    }
    #[tokio::test]
    async fn other_files_external_paths_and_incomplete_exports_are_refused() {
        let st = state("guards");
        let path = st.workspace.logs_dir().join("alllogs_synthetic.txt");
        std::fs::write(&path, b"synthetic").unwrap();
        completed(&st, &path);
        for candidate in [
            st.workspace.root.join("secrets.json"),
            st.workspace.logs_dir().join("other.txt"),
            st.workspace.root.join("alllogs_outside.txt"),
        ] {
            std::fs::write(&candidate, b"synthetic").unwrap();
            assert_eq!(
                download(State(st.clone()), Json(json!({"path":candidate})))
                    .await
                    .status(),
                StatusCode::FORBIDDEN
            );
        }
        completed(&st, &st.workspace.root.join("alllogs_outside.txt"));
        assert_eq!(
            download(
                State(st.clone()),
                Json(json!({"path":st.workspace.root.join("alllogs_outside.txt")}))
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        completed(&st, &path);
        st.update_transient(
            crate::coalesce::Sections::one(crate::coalesce::Section::Scalanie),
            |s| s.scalanie.aktywne = true,
        );
        assert_eq!(
            download(State(st.clone()), Json(json!({"path":path})))
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        std::fs::remove_dir_all(&st.workspace.root).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn junction_output_is_refused_even_when_pointing_inside_workspace() {
        let st = state("junction");
        let link = st.workspace.root.join("linked");
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(st.workspace.logs_dir())
            .output()
            .unwrap();
        assert!(
            status.status.success(),
            "synthetic junction creation failed"
        );
        assert!(no_reparse(&link).is_err());
        // Delete only the link with native API, never its target recursively.
        std::fs::remove_dir(&link).unwrap();
        std::fs::remove_dir_all(&st.workspace.root).unwrap();
    }
}
