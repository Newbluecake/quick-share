use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use http_body_util::BodyExt;
use quick_share_web::{
    ShareCatalog, WebAccess, WebProgressEvent, ZipService, build_browser_router_with_progress,
    build_read_only_router,
};
use std::{fs, sync::Arc, time::Duration};
use tempfile::tempdir;
use tower::ServiceExt;

fn fixture() -> (tempfile::TempDir, Arc<ShareCatalog>) {
    let root = tempdir().expect("catalog root");
    fs::write(root.path().join("hello.txt"), b"hello world").expect("file");
    fs::write(
        root.path().join("dangerous.html"),
        b"<script>document.cookie</script>",
    )
    .expect("HTML file");
    fs::create_dir(root.path().join("docs")).expect("directory");
    fs::create_dir(root.path().join(".quick-share-web-upload")).expect("staging directory");
    fs::write(
        root.path()
            .join(".quick-share-web-upload")
            .join("secret.part"),
        b"partial",
    )
    .expect("partial staging");
    fs::write(root.path().join("docs").join("unicode-.txt"), b"nested").expect("nested file");
    let catalog = ShareCatalog::from_paths(&[root.path().to_path_buf()]).expect("catalog");
    (root, Arc::new(catalog))
}

fn token_uri(path: &str, token: &str) -> String {
    format!("{path}?token={token}")
}

async fn body_bytes(response: axum::response::Response) -> bytes::Bytes {
    response
        .into_body()
        .collect()
        .await
        .expect("response body")
        .to_bytes()
}

#[tokio::test]
async fn catalog_lists_only_public_ids_and_downloads_single_and_nested_files() {
    let (_root, catalog) = fixture();
    let hello = catalog
        .entries()
        .iter()
        .find(|entry| entry.display_path().ends_with("hello.txt"))
        .expect("hello entry");
    let hello_id = hello.id().to_string();
    let (access, token) = WebAccess::generate(Duration::from_secs(60), None).expect("access");
    let app = build_read_only_router(Arc::clone(&catalog), Arc::new(access));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(token_uri("/api/catalog", token.expose()))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("catalog response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::X_CONTENT_TYPE_OPTIONS],
        "nosniff"
    );
    assert!(
        response
            .headers()
            .contains_key(header::CONTENT_SECURITY_POLICY)
    );
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let listing = String::from_utf8(body_bytes(response).await.to_vec()).expect("JSON");
    assert!(listing.contains("hello.txt"));
    assert!(listing.contains("\"mediaType\":\"text/plain\""));
    assert!(listing.contains("unicode-�.txt"));
    assert!(!listing.contains("secret.part"));
    assert!(!listing.contains(".quick-share-web-upload"));
    assert!(!listing.contains(&root_path_marker()));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(token_uri(
                    &format!("/api/download/{hello_id}"),
                    token.expose(),
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("download response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::ACCEPT_RANGES], "bytes");
    assert!(
        response.headers()[header::CONTENT_DISPOSITION]
            .to_str()
            .expect("content disposition")
            .contains("filename*=")
    );
    assert_eq!(body_bytes(response).await.as_ref(), b"hello world");

    let preview = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(token_uri(
                    &format!("/api/preview/{hello_id}"),
                    token.expose(),
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("preview response");
    assert_eq!(preview.status(), StatusCode::OK);
    assert!(
        preview.headers()[header::CONTENT_DISPOSITION]
            .to_str()
            .expect("content disposition")
            .starts_with("inline;")
    );
    assert_eq!(body_bytes(preview).await.as_ref(), b"hello world");

    let html_id = catalog
        .entries()
        .iter()
        .find(|entry| entry.display_path().ends_with("dangerous.html"))
        .expect("HTML entry")
        .id();
    let denied = app
        .oneshot(
            Request::builder()
                .uri(token_uri(
                    &format!("/api/preview/{html_id}"),
                    token.expose(),
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("HTML preview response");
    assert_eq!(denied.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
}

// A stable marker catches accidental absolute-path serialization without depending on temp paths.
fn root_path_marker() -> String {
    std::env::temp_dir().display().to_string()
}

#[tokio::test]
async fn in_memory_text_catalog_is_bounded_to_a_safe_name_and_never_debugs_content() {
    assert!(ShareCatalog::from_text("..", Arc::from(b"secret".as_slice())).is_err());
    assert!(ShareCatalog::from_text("CON", Arc::from(b"secret".as_slice())).is_err());
    let catalog = Arc::new(
        ShareCatalog::from_text("quick-share-text.txt", Arc::from(b"hello".as_slice()))
            .expect("memory catalog"),
    );
    assert!(!format!("{catalog:?}").contains("hello"));
    let id = catalog.entries()[0].id().to_string();
    let (access, token) = WebAccess::generate(Duration::from_secs(60), None).expect("access");
    let response = build_read_only_router(catalog, Arc::new(access))
        .oneshot(
            Request::builder()
                .uri(token_uri(&format!("/api/download/{id}"), token.expose()))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_bytes(response).await.as_ref(), b"hello");
}

#[tokio::test]
async fn token_expiry_quota_unknown_ids_and_path_shaped_ids_fail_closed() {
    let (_root, catalog) = fixture();
    let file_id = catalog
        .entries()
        .iter()
        .find(|entry| entry.display_path().ends_with("hello.txt"))
        .expect("file")
        .id()
        .to_string();
    let (access, token) = WebAccess::generate(Duration::from_secs(60), Some(1)).expect("access");
    let app = build_read_only_router(Arc::clone(&catalog), Arc::new(access));

    for uri in [
        "/api/catalog".to_owned(),
        "/api/catalog?token=wrong".to_owned(),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    let first = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(token_uri(
                    &format!("/api/download/{file_id}"),
                    token.expose(),
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("first download");
    assert_eq!(first.status(), StatusCode::OK);

    let exhausted = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(token_uri(
                    &format!("/api/download/{file_id}"),
                    token.expose(),
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("exhausted response");
    assert_eq!(exhausted.status(), StatusCode::TOO_MANY_REQUESTS);

    for id in [
        "00000000-0000-0000-0000-000000000000",
        "..",
        "%2e%2e",
        "%252e%252e",
        "%5cwindows%5csystem32",
        "%2fetc%2fpasswd",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(token_uri(&format!("/api/download/{id}"), token.expose()))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("unknown response");
        assert!(matches!(
            response.status(),
            StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND | StatusCode::TOO_MANY_REQUESTS
        ));
    }

    let (expired, expired_token) =
        WebAccess::generate(Duration::ZERO, None).expect("expired access");
    let expired_app = build_read_only_router(catalog, Arc::new(expired));
    let response = expired_app
        .oneshot(
            Request::builder()
                .uri(token_uri("/api/catalog", expired_token.expose()))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("expired response");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn range_is_single_bounded_and_streaming_body_does_not_prebuffer_the_file() {
    let root = tempdir().expect("root");
    let large = root.path().join("large.bin");
    let payload = vec![0x5a; 8 * 1024 * 1024];
    fs::write(&large, &payload).expect("large file");
    let catalog = Arc::new(ShareCatalog::from_paths(&[large]).expect("catalog"));
    let file_id = catalog.entries()[0].id().to_string();
    let (access, token) = WebAccess::generate(Duration::from_secs(60), None).expect("access");
    let app = build_read_only_router(catalog, Arc::new(access));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(token_uri(
                    &format!("/api/download/{file_id}"),
                    token.expose(),
                ))
                .header(header::RANGE, "bytes=100-199")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("range response");
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(response.headers()[header::CONTENT_LENGTH], "100");
    assert_eq!(
        response.headers()[header::CONTENT_RANGE],
        format!("bytes 100-199/{}", payload.len())
    );
    assert_eq!(body_bytes(response).await.as_ref(), &payload[100..200]);

    let invalid = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(token_uri(
                    &format!("/api/download/{file_id}"),
                    token.expose(),
                ))
                .header(header::RANGE, "bytes=0-1,4-5")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("invalid range");
    assert_eq!(invalid.status(), StatusCode::RANGE_NOT_SATISFIABLE);

    let response = app
        .oneshot(
            Request::builder()
                .uri(token_uri(
                    &format!("/api/download/{file_id}"),
                    token.expose(),
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("stream response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_LENGTH],
        payload.len().to_string()
    );
    let mut body = response.into_body();
    let first = body
        .frame()
        .await
        .expect("first frame")
        .expect("body frame")
        .into_data()
        .expect("data frame");
    assert!(first.len() < payload.len());
}

#[tokio::test]
async fn completed_download_emits_bounded_terminal_progress_events() {
    let (root, catalog) = fixture();
    let entry_id = catalog
        .entries()
        .iter()
        .find(|entry| entry.display_path().ends_with("hello.txt"))
        .expect("entry")
        .id()
        .clone();
    let (access, token) = WebAccess::generate(Duration::from_secs(60), None).expect("access");
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel(8);
    let app = build_browser_router_with_progress(
        catalog,
        Arc::new(access),
        None,
        Arc::new(ZipService::new(1, 2).expect("ZIP")),
        progress_tx,
    );
    let response = app
        .oneshot(
            Request::builder()
                .uri(token_uri(
                    &format!("/api/download/{entry_id}"),
                    token.expose(),
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    assert_eq!(body.as_ref(), b"hello world");
    let mut events = Vec::new();
    while let Ok(event) = progress_rx.try_recv() {
        events.push(event);
    }
    assert!(matches!(
        events.first(),
        Some(WebProgressEvent::DownloadStarted {
            total_bytes: 11,
            ..
        })
    ));
    assert!(matches!(
        events.last(),
        Some(WebProgressEvent::DownloadCompleted {
            total_bytes: 11,
            ..
        })
    ));
    assert!(events.len() <= 3);
    drop(root);
}

#[cfg(unix)]
#[tokio::test]
async fn symlink_roots_internal_links_and_post_catalog_swaps_never_escape() {
    use std::os::unix::fs::symlink;

    let outside = tempdir().expect("outside");
    fs::write(outside.path().join("secret.txt"), b"secret").expect("secret");
    let root = tempdir().expect("root");
    fs::write(root.path().join("safe.txt"), b"safe").expect("safe");
    symlink(
        outside.path().join("secret.txt"),
        root.path().join("escape.txt"),
    )
    .expect("internal link");
    let root_link = root.path().with_extension("link");
    symlink(root.path(), &root_link).expect("root link");
    assert!(ShareCatalog::from_paths(std::slice::from_ref(&root_link)).is_err());

    let catalog =
        Arc::new(ShareCatalog::from_paths(&[root.path().to_path_buf()]).expect("catalog"));
    assert!(
        catalog
            .entries()
            .iter()
            .all(|entry| !entry.display_path().ends_with("escape.txt"))
    );
    let safe = catalog
        .entries()
        .iter()
        .find(|entry| entry.display_path().ends_with("safe.txt"))
        .expect("safe entry");
    let safe_id = safe.id().to_string();
    fs::remove_file(root.path().join("safe.txt")).expect("remove safe");
    symlink(
        outside.path().join("secret.txt"),
        root.path().join("safe.txt"),
    )
    .expect("swap link");

    let (access, token) = WebAccess::generate(Duration::from_secs(60), None).expect("access");
    let app = build_read_only_router(catalog, Arc::new(access));
    let response = app
        .oneshot(
            Request::builder()
                .uri(token_uri(
                    &format!("/api/download/{safe_id}"),
                    token.expose(),
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert!(matches!(
        response.status(),
        StatusCode::NOT_FOUND | StatusCode::CONFLICT
    ));
    assert_ne!(body_bytes(response).await.as_ref(), b"secret");

    fs::remove_file(root_link).expect("remove link");
}
