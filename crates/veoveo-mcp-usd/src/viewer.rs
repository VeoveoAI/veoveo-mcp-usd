//! The viewer's built files on the server's public route, which the Hub reaches at
//! `/_veoveo/viewers/usd/` on a file's origin. They hold nothing private. The browser
//! revalidates them, and takes the gzip copy `openshell/run` makes of the WASM.
use std::path::Path;

use axum::{Router, http::header};
use tower_http::{services::ServeDir, set_header::SetResponseHeaderLayer};

/// The route's router, under the server's mount, for the files in `directory`.
pub fn router(directory: &Path) -> Router {
    Router::new()
        .nest_service("/viewer", ServeDir::new(directory).precompressed_gzip())
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            header::HeaderValue::from_static("no-cache"),
        ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode, header::ACCEPT_ENCODING},
    };
    use tower::ServiceExt as _;

    async fn get(
        directory: &Path,
        path: &str,
        gzip: bool,
    ) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
        let mut request = Request::builder().uri(path);
        if gzip {
            request = request.header(ACCEPT_ENCODING, "gzip");
        }
        let response = router(directory)
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, headers, body.to_vec())
    }

    #[tokio::test]
    async fn the_route_serves_the_page_its_files_and_their_gzip_copies() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("index.html"), "<!doctype html>").unwrap();
        std::fs::write(directory.path().join("v_bg.wasm"), "wasm").unwrap();
        std::fs::write(directory.path().join("v_bg.wasm.gz"), "gz").unwrap();

        let (status, headers, body) = get(directory.path(), "/viewer/?file=/a/a.usd", false).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, b"<!doctype html>");
        assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
        assert!(
            headers[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with("text/html")
        );

        let (_, headers, body) = get(directory.path(), "/viewer/v_bg.wasm", false).await;
        assert_eq!(body, b"wasm");
        assert_eq!(headers[header::CONTENT_TYPE], "application/wasm");

        let (_, headers, body) = get(directory.path(), "/viewer/v_bg.wasm", true).await;
        assert_eq!(body, b"gz");
        assert_eq!(headers[header::CONTENT_ENCODING], "gzip");
        assert_eq!(headers[header::CONTENT_TYPE], "application/wasm");

        let (status, ..) = get(directory.path(), "/viewer/../secret", false).await;
        assert_ne!(status, StatusCode::OK);
    }
}
