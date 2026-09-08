//! Browser-origin boundary for the local API, including WebSocket upgrades.
//! This is not authentication of native clients on the same computer.
use axum::extract::Request;
use axum::http::{
    header::{HOST, ORIGIN},
    HeaderMap, HeaderValue, StatusCode, Uri,
};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

pub(crate) fn allowed(origin: &HeaderValue) -> bool {
    let Ok(raw) = origin.to_str() else {
        return false;
    };
    let Ok(uri) = raw.parse::<Uri>() else {
        return false;
    };
    let (Some(scheme), Some(authority)) = (uri.scheme_str(), uri.authority()) else {
        return false;
    };
    // A serialized Origin is scheme + authority, never credentials, a path,
    // query or fragment. Checking the whole value avoids prefix lookalikes.
    if raw != format!("{scheme}://{authority}") {
        return false;
    }
    let host = authority.host();
    let Some(suffix) = authority.as_str().strip_prefix(host) else {
        return false;
    };
    if !suffix.is_empty()
        && !suffix.strip_prefix(':').is_some_and(|port| {
            !port.is_empty()
                && port.bytes().all(|b| b.is_ascii_digit())
                && port.parse::<u16>().is_ok()
        })
    {
        return false;
    }
    match scheme {
        "tauri" => host.eq_ignore_ascii_case("localhost") && suffix.is_empty(),
        "http" | "https" => ["localhost", "127.0.0.1", "[::1]", "tauri.localhost"]
            .iter()
            .any(|trusted| host.eq_ignore_ascii_case(trusted)),
        _ => false,
    }
}

fn local_authority(value: &str) -> bool {
    HeaderValue::from_str(&format!("http://{value}")).is_ok_and(|origin| allowed(&origin))
}

fn request_allowed(headers: &HeaderMap) -> bool {
    // Same-origin GET can omit Origin after DNS rebinding. The local server
    // therefore also rejects an explicit foreign Host, including for /api/state.
    let mut hosts = headers.get_all(HOST).iter();
    if let Some(host) = hosts.next() {
        if hosts.next().is_some() || !host.to_str().is_ok_and(local_authority) {
            return false;
        }
    }
    let mut origins = headers.get_all(ORIGIN).iter();
    match origins.next() {
        // Preserve the desktop/CLI contract. Local native clients are trusted;
        // absence of Origin must never be described as account authentication.
        None => true,
        Some(origin) => origins.next().is_none() && allowed(origin),
    }
}

pub(crate) async fn guard(request: Request, next: Next) -> Response {
    if !request_allowed(request.headers())
        || request
            .uri()
            .authority()
            .is_some_and(|a| !local_authority(a.as_str()))
    {
        return (StatusCode::FORBIDDEN, "Untrusted browser origin").into_response();
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_local_and_desktop_origins_are_supported() {
        for value in [
            "http://localhost",
            "http://localhost:5180",
            "http://127.0.0.1:8787",
            "https://[::1]:8787",
            "tauri://localhost",
            "http://tauri.localhost",
        ] {
            assert!(allowed(&HeaderValue::from_str(value).unwrap()), "{value}");
        }
    }

    #[test]
    fn lookalikes_userinfo_paths_and_foreign_schemes_are_rejected() {
        for value in [
            "https://localhost.attacker.example",
            "http://127.0.0.1.attacker.example",
            "http://attacker.example//localhost",
            "http://localhost@attacker.example",
            "http://attacker@localhost",
            "tauri://attacker.example",
            "tauri://localhost:1",
            "http://localhost/",
            "http://localhost?x=1",
            "http://localhost#x",
            "http://localhost:65536",
            "http://localhost:abc",
            "null",
            "file://localhost",
            "http://localhost http://attacker.example",
        ] {
            assert!(!allowed(&HeaderValue::from_str(value).unwrap()), "{value}");
        }
    }

    #[test]
    fn duplicate_origin_is_rejected_and_native_absence_is_preserved() {
        let mut headers = HeaderMap::new();
        assert!(request_allowed(&headers));
        headers.append(ORIGIN, HeaderValue::from_static("http://localhost"));
        assert!(request_allowed(&headers));
        headers.append(ORIGIN, HeaderValue::from_static("https://attacker.example"));
        assert!(!request_allowed(&headers));
    }

    #[test]
    fn foreign_host_cannot_read_private_data_by_omitting_origin() {
        let mut headers = HeaderMap::new();
        headers.insert(
            HOST,
            HeaderValue::from_static("localhost.attacker.example:8787"),
        );
        assert!(!request_allowed(&headers));
        headers.insert(HOST, HeaderValue::from_static("127.0.0.1:8787"));
        assert!(request_allowed(&headers));
        headers.append(HOST, HeaderValue::from_static("localhost:8787"));
        assert!(!request_allowed(&headers));
    }

    #[tokio::test]
    async fn rejected_origins_never_reach_rest_or_websocket_handlers() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let app = axum::Router::new()
            .fallback(move || {
                let counted = counted.clone();
                async move {
                    counted.fetch_add(1, Ordering::Relaxed);
                    StatusCode::NO_CONTENT
                }
            })
            .layer(axum::middleware::from_fn(guard));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        for (method, path, origin, host, expected) in [
            (
                "POST",
                "/api/fs/download",
                Some("https://localhost.attacker.example"),
                address.to_string(),
                "403",
            ),
            (
                "GET",
                "/ws",
                Some("https://attacker.example"),
                address.to_string(),
                "403",
            ),
            (
                "GET",
                "/api/state",
                None,
                "rebound.attacker.example:8787".to_owned(),
                "403",
            ),
            (
                "GET",
                "/api/state",
                Some("http://127.0.0.1:8787"),
                address.to_string(),
                "204",
            ),
        ] {
            let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
            let origin = origin
                .map(|o| format!("Origin: {o}\r\n"))
                .unwrap_or_default();
            let request = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\n{origin}Content-Length: 0\r\nConnection: close\r\n\r\n");
            socket.write_all(request.as_bytes()).await.unwrap();
            let mut response = Vec::new();
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                socket.read_to_end(&mut response),
            )
            .await
            .unwrap()
            .unwrap();
            assert!(String::from_utf8(response)
                .unwrap()
                .starts_with(&format!("HTTP/1.1 {expected}")));
        }
        server.abort();
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }
}
