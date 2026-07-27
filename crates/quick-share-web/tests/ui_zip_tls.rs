use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use http_body_util::BodyExt;
use quick_share_web::{
    ExplicitHttp, PreparedWebSecurity, ShareCatalog, WebAccess, WebSecurity, ZipService,
    build_browser_router, endpoint_info,
};
use std::{
    fs,
    io::{Cursor, Read},
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tempfile::tempdir;
use tower::ServiceExt;

fn browser_fixture() -> (
    tempfile::TempDir,
    Arc<ShareCatalog>,
    Arc<WebAccess>,
    quick_share_web::AccessToken,
) {
    let root = tempdir().expect("root");
    fs::write(root.path().join("hello.txt"), b"hello").expect("file");
    #[cfg(unix)]
    {
        fs::write(root.path().join("CON"), b"reserved").expect("reserved file");
        fs::write(root.path().join("a:b.txt"), b"colon").expect("colon file");
    }
    fs::create_dir(root.path().join("empty")).expect("empty directory");
    fs::create_dir(root.path().join("资料")).expect("unicode directory");
    fs::write(root.path().join("资料").join("你好.txt"), b"unicode").expect("unicode file");
    let catalog =
        Arc::new(ShareCatalog::from_paths(&[root.path().to_path_buf()]).expect("catalog"));
    let (access, token) = WebAccess::generate(Duration::from_secs(60), None).expect("access");
    (root, catalog, Arc::new(access), token)
}

#[tokio::test]
async fn embedded_ui_is_offline_safe_responsive_and_uses_dom_text_not_html_injection() {
    let (_root, catalog, access, token) = browser_fixture();
    let app = build_browser_router(
        catalog,
        access,
        None,
        Arc::new(ZipService::new(1, 4).expect("zip service")),
    );
    let html = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/?token={}", token.expose()))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("HTML");
    assert_eq!(html.status(), StatusCode::OK);
    assert_eq!(
        html.headers()[header::CONTENT_SECURITY_POLICY],
        "default-src 'self'; base-uri 'none'; object-src 'none'; frame-ancestors 'none'; form-action 'self'; img-src 'self' data:; style-src 'self'; script-src 'self'; connect-src 'self'"
    );
    let html = String::from_utf8(
        html.into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes()
            .to_vec(),
    )
    .expect("UTF-8");
    assert!(html.contains("name=\"viewport\""));
    assert!(html.contains("id=\"catalog\""));
    assert!(html.contains("id=\"drop-zone\""));
    assert!(html.contains("/assets/app.css"));
    assert!(html.contains("/assets/app.js"));
    assert!(!html.contains("https://"));
    assert!(!html.contains("http://"));

    let css = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/assets/app.css")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("CSS");
    let css = String::from_utf8(
        css.into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes()
            .to_vec(),
    )
    .expect("UTF-8");
    assert!(css.contains("@media"));
    assert!(css.contains("--qs-"));

    let javascript = app
        .oneshot(
            Request::builder()
                .uri("/assets/app.js")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("JavaScript");
    let javascript = String::from_utf8(
        javascript
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes()
            .to_vec(),
    )
    .expect("UTF-8");
    assert!(javascript.contains("textContent"));
    assert!(javascript.contains("XMLHttpRequest"));
    assert!(!javascript.contains("innerHTML"));
    assert!(!javascript.contains("eval("));
    assert!(!javascript.contains("https://"));
}

#[tokio::test]
async fn download_all_streams_unicode_files_and_empty_directories_as_zip() {
    let (_root, catalog, access, token) = browser_fixture();
    let zip_service = Arc::new(ZipService::new(1, 2).expect("zip service"));
    let app = build_browser_router(catalog, access, None, Arc::clone(&zip_service));
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/download-all?token={}", token.expose()))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("ZIP response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/zip");
    assert!(
        response.headers()[header::CONTENT_DISPOSITION]
            .to_str()
            .expect("header")
            .contains("quick-share.zip")
    );
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("ZIP body")
        .to_bytes();
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("ZIP archive");
    let names = (0..archive.len())
        .map(|index| archive.by_index(index).expect("entry").name().to_owned())
        .collect::<Vec<_>>();
    assert!(names.iter().any(|name| name.ends_with("hello.txt")));
    assert!(names.iter().any(|name| name.ends_with("empty/")));
    #[cfg(unix)]
    {
        assert!(names.iter().any(|name| name.ends_with("_CON")));
        assert!(names.iter().any(|name| name.ends_with("a_b.txt")));
        assert!(!names.iter().any(|name| name.ends_with("/CON")));
    }
    let unicode_index = names
        .iter()
        .position(|name| name.ends_with("资料/你好.txt"))
        .expect("Unicode entry");
    let mut unicode = archive.by_index(unicode_index).expect("Unicode file");
    let mut contents = Vec::new();
    unicode.read_to_end(&mut contents).expect("read ZIP file");
    assert_eq!(contents, b"unicode");
    assert_eq!(zip_service.active_workers(), 0);
}

#[tokio::test]
async fn dropping_zip_body_cancels_the_bounded_blocking_worker() {
    let root = tempdir().expect("root");
    let large = root.path().join("large.bin");
    fs::write(&large, vec![0x55; 32 * 1024 * 1024]).expect("large file");
    let catalog = Arc::new(ShareCatalog::from_paths(&[large]).expect("catalog"));
    let (access, token) = WebAccess::generate(Duration::from_secs(60), None).expect("access");
    let zip_service = Arc::new(ZipService::new(1, 1).expect("zip service"));
    let app = build_browser_router(catalog, Arc::new(access), None, Arc::clone(&zip_service));
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/download-all?token={}", token.expose()))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("ZIP response");
    let mut body = response.into_body();
    let _ = body.frame().await.expect("first ZIP frame");
    drop(body);
    tokio::time::timeout(Duration::from_secs(2), async {
        while zip_service.active_workers() != 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("worker cancellation");
}

#[tokio::test]
async fn tls_is_default_user_pem_is_supported_and_http_requires_explicit_opt_in() {
    let ip = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10));
    let first = PreparedWebSecurity::prepare(WebSecurity::SelfSigned, &[ip])
        .await
        .expect("self-signed TLS");
    let second = PreparedWebSecurity::prepare(WebSecurity::SelfSigned, &[ip])
        .await
        .expect("second TLS");
    assert_eq!(first.scheme(), "https");
    assert!(first.is_self_signed());
    assert_eq!(
        first.certificate_fingerprint().expect("fingerprint").len(),
        64
    );
    assert_ne!(
        first.certificate_fingerprint(),
        second.certificate_fingerprint()
    );

    let certs = tempdir().expect("certs");
    let params = rcgen::CertificateParams::new(vec!["localhost".to_owned()])
        .expect("certificate parameters");
    let key = rcgen::KeyPair::generate().expect("user key");
    let certificate = params.self_signed(&key).expect("user certificate");
    fs::write(certs.path().join("cert.pem"), certificate.pem()).expect("cert PEM");
    fs::write(certs.path().join("key.pem"), key.serialize_pem()).expect("key PEM");
    let provided = PreparedWebSecurity::prepare(
        WebSecurity::UserProvided {
            certificate: certs.path().join("cert.pem"),
            private_key: certs.path().join("key.pem"),
        },
        &[ip],
    )
    .await
    .expect("provided TLS");
    assert_eq!(provided.scheme(), "https");
    assert!(!provided.is_self_signed());

    assert!(ExplicitHttp::new(false).is_err());
    let http = PreparedWebSecurity::prepare(
        WebSecurity::Http(ExplicitHttp::new(true).expect("explicit HTTP")),
        &[ip],
    )
    .await
    .expect("HTTP");
    assert_eq!(http.scheme(), "http");
    assert!(http.certificate_fingerprint().is_none());
}

#[tokio::test]
async fn endpoint_qr_links_and_commands_match_the_actual_listener_and_security() {
    let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let security = PreparedWebSecurity::prepare(WebSecurity::SelfSigned, &[ip])
        .await
        .expect("TLS");
    let (_access, token) = WebAccess::generate(Duration::from_secs(60), Some(3)).expect("access");
    let endpoint = endpoint_info(
        SocketAddr::new(ip, 45678),
        ip,
        &security,
        token.expose(),
        Duration::from_secs(60),
        Some(3),
    )
    .expect("endpoint");
    assert_eq!(
        endpoint.url,
        format!("https://127.0.0.1:45678/?token={}", token.expose())
    );
    assert!(endpoint.qr.contains('█'));
    assert!(endpoint.curl.contains(&endpoint.url));
    assert!(endpoint.curl.contains("--insecure"));
    assert!(endpoint.wget.contains(&endpoint.url));
    assert_eq!(endpoint.expires_in, Duration::from_secs(60));
    assert_eq!(endpoint.max_downloads, Some(3));
}
