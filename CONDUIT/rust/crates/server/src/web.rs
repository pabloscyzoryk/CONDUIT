
use axum::body::Body;
use axum::http::{header, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;
use std::path::{Path, PathBuf};

#[derive(RustEmbed)]
#[folder = "web/"]
struct Assets;

#[derive(Debug, Clone)]
pub enum WebSource {
    /// pliki z dysku (praca nad UI, awaryjna podmiana)
    Disk(PathBuf),
    /// zasoby wkompilowane w binarkę
    Embedded,
}

impl WebSource {
    /// Wybiera źródło wg opisanej wyżej kolejności.
    pub fn resolve(explicit: Option<PathBuf>, exe_dir: &Path) -> WebSource {
        if let Some(p) = explicit {
            if p.join("index.html").is_file() {
                return WebSource::Disk(p);
            }
            tracing::warn!(katalog = %p.display(), "wskazany katalog nie ma index.html — używam zasobów wbudowanych");
        }
        let obok = exe_dir.join("web");
        if obok.join("index.html").is_file() {
            return WebSource::Disk(obok);
        }
        WebSource::Embedded
    }

    pub fn describe(&self) -> String {
        match self {
            WebSource::Disk(p) => format!("dysk: {}", p.display()),
            WebSource::Embedded => "wbudowane w binarkę".into(),
        }
    }

    /// Czy w ogóle mamy co serwować? Placeholder nie liczy się jako interfejs.
    pub fn has_app(&self) -> bool {
        match self {
            WebSource::Disk(p) => p.join("index.html").is_file(),
            WebSource::Embedded => Assets::get("index.html")
                .map(|f| !f.data.starts_with(b"<!-- PLACEHOLDER"))
                .unwrap_or(false),
        }
    }

    fn load(&self, rel: &str) -> Option<(Vec<u8>, &'static str)> {
        let mime = mime_guess::from_path(rel).first_or_octet_stream();
        // `first_or_octet_stream` zwraca `Mime`; potrzebujemy statycznego napisu
        let mime: &'static str = Box::leak(mime.essence_str().to_string().into_boxed_str());
        match self {
            WebSource::Disk(root) => {
                let p = safe_join(root, rel)?;
                std::fs::read(p).ok().map(|d| (d, mime))
            }
            WebSource::Embedded => Assets::get(rel).map(|f| (f.data.to_vec(), mime)),
        }
    }

    fn index(&self) -> Option<Vec<u8>> {
        self.load("index.html").map(|(d, _)| d)
    }
}

/// Blokuje wyjście poza katalog (`..`, ścieżki bezwzględne, dyski).
fn safe_join(root: &Path, rel: &str) -> Option<PathBuf> {
    let mut out = root.to_path_buf();
    for part in rel.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." || part.contains(':') || part.contains('\\') {
            return None;
        }
        out.push(part);
    }
    Some(out)
}

/// Handler statyków z zachowaniem trasowania po stronie klienta (SPA):
/// nieznana ścieżka, która nie wygląda na plik, dostaje `index.html`.
pub async fn serve(source: WebSource, uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let rel = if path.is_empty() { "index.html" } else { path };

    if let Some((data, mime)) = source.load(rel) {
        return with_headers(data, mime, rel);
    }

    // trasowanie SPA — ale tylko dla ścieżek bez rozszerzenia,
    // żeby brakujący `main.js` zwracał 404, a nie stronę HTML
    let wyglada_na_plik = rel
        .rsplit('/')
        .next()
        .map(|s| s.contains('.'))
        .unwrap_or(false);
    if !wyglada_na_plik {
        if let Some(data) = source.index() {
            return with_headers(data, "text/html", "index.html");
        }
    }

    (StatusCode::NOT_FOUND, "nie znaleziono").into_response()
}

fn with_headers(data: Vec<u8>, mime: &str, rel: &str) -> Response {
    // Vite stempluje pliki z `assets/` skrótem treści → można je cache'ować
    // na zawsze. `index.html` NIGDY, bo inaczej po aktualizacji przeglądarka
    // trzymałaby stary HTML wskazujący nieistniejące już pliki.
    let cache = if rel.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    let mut res = Response::new(Body::from(data));
    let h = res.headers_mut();
    if let Ok(v) = HeaderValue::from_str(mime) {
        h.insert(header::CONTENT_TYPE, v);
    }
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nie_da_sie_wyjsc_poza_katalog() {
        let root = Path::new("C:/app/web");
        assert!(safe_join(root, "../../secret.txt").is_none());
        assert!(safe_join(root, "..\\secret.txt").is_none());
        assert!(safe_join(root, "C:/windows/win.ini").is_none());
        assert!(safe_join(root, "assets/index.js").is_some());
    }

    #[test]
    fn zasoby_z_hashem_sa_cachowane_a_html_nie() {
        let r = with_headers(b"x".to_vec(), "text/javascript", "assets/main-a1b2.js");
        assert!(r.headers()[header::CACHE_CONTROL]
            .to_str()
            .unwrap()
            .contains("immutable"));
        let r = with_headers(b"x".to_vec(), "text/html", "index.html");
        assert_eq!(r.headers()[header::CACHE_CONTROL], "no-cache");
    }

    #[test]
    fn wybor_zrodla_schodzi_do_wbudowanych() {
        let brak = PathBuf::from("C:/nie/ma/takiego/katalogu");
        let src = WebSource::resolve(Some(brak), Path::new("C:/tez/nie/ma"));
        assert!(matches!(src, WebSource::Embedded));
    }
}
