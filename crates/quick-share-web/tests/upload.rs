use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use http_body_util::BodyExt;
use quick_share_core::config::ConflictPolicy;
use quick_share_web::{ShareCatalog, UploadConfig, UploadService, WebAccess, build_web_router};
use std::{fs, sync::Arc, time::Duration};
use tempfile::tempdir;
use tower::ServiceExt;

const BOUNDARY: &str = "quick-share-test-boundary";

fn multipart(parts: &[(&str, &[u8])]) -> Vec<u8> {
    let mut body = Vec::new();
    for (name, bytes) in parts {
        body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
        body.extend_from_slice(
            format!(
                "Content-Disposition: form-data; name=\"files\"; filename=\"{name}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    body
}

fn upload_request(token: &str, password: Option<&str>, body: Body) -> Request<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri(format!("/api/upload?token={token}"))
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={BOUNDARY}"),
        );
    if let Some(password) = password {
        builder = builder.header("x-quick-share-upload-password", password);
    }
    builder.body(body).expect("request")
}

fn app(
    output: &std::path::Path,
    access: Arc<WebAccess>,
    password: Option<&str>,
    configure: impl FnOnce(&mut UploadConfig),
) -> axum::Router {
    let shared = output.join("shared.txt");
    fs::write(&shared, b"download").expect("shared file");
    let catalog = Arc::new(ShareCatalog::from_paths(&[shared]).expect("catalog"));
    let mut config = UploadConfig {
        output_root: output.to_path_buf(),
        max_file_bytes: 1024,
        max_total_bytes: 2048,
        max_body_bytes: 4096,
        conflict: ConflictPolicy::Rename,
        max_uploads_per_window: 3,
        rate_window: Duration::from_secs(60),
        max_concurrent_uploads: 2,
    };
    configure(&mut config);
    let upload = Arc::new(UploadService::new(config, password).expect("upload service"));
    build_web_router(catalog, access, Some(upload))
}

#[tokio::test]
async fn multipart_upload_streams_multiple_files_and_renames_conflicts() {
    let root = tempdir().expect("root");
    let output = root.path().join("received");
    fs::create_dir(&output).expect("output");
    let (access, token) = WebAccess::generate(Duration::from_secs(60), None).expect("access");
    let app = app(&output, Arc::new(access), Some("secret"), |_| {});

    let response = app
        .clone()
        .oneshot(upload_request(
            token.expose(),
            Some("secret"),
            Body::from(multipart(&[("alpha.txt", b"alpha"), ("beta.txt", b"beta")])),
        ))
        .await
        .expect("upload");
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(fs::read(output.join("alpha.txt")).expect("alpha"), b"alpha");
    assert_eq!(fs::read(output.join("beta.txt")).expect("beta"), b"beta");

    let response = app
        .oneshot(upload_request(
            token.expose(),
            Some("secret"),
            Body::from(multipart(&[("alpha.txt", b"second")])),
        ))
        .await
        .expect("rename upload");
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        fs::read(output.join("alpha (1).txt")).expect("renamed"),
        b"second"
    );
    assert!(
        fs::read_dir(output.join(".quick-share-web-upload"))
            .expect("staging")
            .next()
            .is_none()
    );
}

#[tokio::test]
async fn upload_requires_access_token_and_optional_password_without_consuming_download_quota() {
    let root = tempdir().expect("root");
    let output = root.path().join("received");
    fs::create_dir(&output).expect("output");
    let (access, token) = WebAccess::generate(Duration::from_secs(60), Some(1)).expect("access");
    let app = app(&output, Arc::new(access), Some("secret"), |_| {});
    let body = || Body::from(multipart(&[("safe.txt", b"safe")]));

    for request in [
        upload_request("wrong", Some("secret"), body()),
        upload_request(token.expose(), None, body()),
        upload_request(token.expose(), Some("wrong"), body()),
    ] {
        let response = app.clone().oneshot(request).await.expect("auth response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    let uploaded = app
        .clone()
        .oneshot(upload_request(token.expose(), Some("secret"), body()))
        .await
        .expect("upload");
    assert_eq!(uploaded.status(), StatusCode::CREATED);

    let rate_limited = app
        .clone()
        .oneshot(upload_request(token.expose(), Some("secret"), body()))
        .await
        .expect("rate-limited upload");
    assert_eq!(rate_limited.status(), StatusCode::TOO_MANY_REQUESTS);

    let listing = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/catalog?token={}", token.expose()))
                .body(Body::empty())
                .expect("listing request"),
        )
        .await
        .expect("listing");
    let listing_bytes = listing
        .into_body()
        .collect()
        .await
        .expect("listing body")
        .to_bytes();
    let listing: serde_json::Value = serde_json::from_slice(&listing_bytes).expect("listing JSON");
    assert_eq!(listing["uploadPasswordRequired"], true);
    let file_id = listing["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .find(|entry| entry["name"] == "shared.txt")
        .expect("shared entry")["id"]
        .as_str()
        .expect("entry ID");
    let first_download = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/download/{file_id}?token={}", token.expose()))
                .body(Body::empty())
                .expect("download"),
        )
        .await
        .expect("download response");
    assert_eq!(first_download.status(), StatusCode::OK);
    let second_download = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/download/{file_id}?token={}", token.expose()))
                .body(Body::empty())
                .expect("download"),
        )
        .await
        .expect("download response");
    assert_eq!(second_download.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn malicious_filenames_and_size_or_rate_limits_fail_without_final_or_staging_files() {
    for name in [
        "../escape.txt",
        "..\\escape.txt",
        "/etc/passwd",
        "CON",
        "bad\u{1b}.txt",
    ] {
        let root = tempdir().expect("root");
        let output = root.path().join("received");
        fs::create_dir(&output).expect("output");
        let (access, token) = WebAccess::generate(Duration::from_secs(60), None).expect("access");
        let app = app(&output, Arc::new(access), None, |_| {});
        let response = app
            .oneshot(upload_request(
                token.expose(),
                None,
                Body::from(multipart(&[(name, b"payload")])),
            ))
            .await
            .expect("invalid filename response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{name:?}");
        assert!(!root.path().join("escape.txt").exists());
    }

    for (parts, configure) in [
        (
            vec![("large.txt", b"12345".as_slice())],
            (4_u64, 20_u64, 4096_usize),
        ),
        (
            vec![
                ("one.txt", b"1234".as_slice()),
                ("two.txt", b"5678".as_slice()),
            ],
            (4_u64, 7_u64, 4096_usize),
        ),
        (
            vec![("body.txt", b"1234567890".as_slice())],
            (20_u64, 20_u64, 100_usize),
        ),
    ] {
        let root = tempdir().expect("root");
        let output = root.path().join("received");
        fs::create_dir(&output).expect("output");
        let (access, token) = WebAccess::generate(Duration::from_secs(60), None).expect("access");
        let app = app(&output, Arc::new(access), None, |config| {
            config.max_file_bytes = configure.0;
            config.max_total_bytes = configure.1;
            config.max_body_bytes = configure.2;
        });
        let response = app
            .oneshot(upload_request(
                token.expose(),
                None,
                Body::from(multipart(&parts)),
            ))
            .await
            .expect("limit response");
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let final_files = fs::read_dir(&output)
            .expect("output listing")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry.file_name() != ".quick-share-web-upload" && entry.file_name() != "shared.txt"
            })
            .count();
        assert_eq!(final_files, 0);
    }

    let root = tempdir().expect("root");
    let output = root.path().join("received");
    fs::create_dir(&output).expect("output");
    let (access, token) = WebAccess::generate(Duration::from_secs(60), None).expect("access");
    let app = app(&output, Arc::new(access), None, |config| {
        config.max_uploads_per_window = 1;
    });
    let first = app
        .clone()
        .oneshot(upload_request(
            token.expose(),
            None,
            Body::from(multipart(&[("one.txt", b"one")])),
        ))
        .await
        .expect("first");
    assert_eq!(first.status(), StatusCode::CREATED);
    let limited = app
        .oneshot(upload_request(
            token.expose(),
            None,
            Body::from(multipart(&[("two.txt", b"two")])),
        ))
        .await
        .expect("limited");
    assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn disconnected_multipart_body_cleans_partial_staging() {
    let root = tempdir().expect("root");
    let output = root.path().join("received");
    fs::create_dir(&output).expect("output");
    let (access, token) = WebAccess::generate(Duration::from_secs(60), None).expect("access");
    let app = app(&output, Arc::new(access), None, |_| {});
    let prefix = format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"files\"; filename=\"partial.txt\"\r\n\r\npartial"
    );
    let stream = futures_util::stream::iter(vec![
        Ok::<_, std::io::Error>(bytes::Bytes::from(prefix)),
        Err(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "client disconnected",
        )),
    ]);
    let response = app
        .oneshot(upload_request(
            token.expose(),
            None,
            Body::from_stream(stream),
        ))
        .await
        .expect("disconnect response");
    assert!(matches!(
        response.status(),
        StatusCode::BAD_REQUEST | StatusCode::INTERNAL_SERVER_ERROR
    ));
    let staging = output.join(".quick-share-web-upload");
    assert!(fs::read_dir(staging).expect("staging").next().is_none());
    assert!(!output.join("partial.txt").exists());
}
