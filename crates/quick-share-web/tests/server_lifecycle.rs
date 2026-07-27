use quick_share_web::{
    ExplicitHttp, PreparedWebSecurity, ShareCatalog, WebAccess, WebSecurity, ZipService,
    build_browser_router, start_web_server,
};
use std::{
    fs,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tempfile::tempdir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use tokio_util::sync::CancellationToken;

async fn http_get(address: SocketAddr, target: &str) -> Vec<u8> {
    let mut stream = tokio::time::timeout(Duration::from_secs(2), TcpStream::connect(address))
        .await
        .expect("connect timeout")
        .expect("connect");
    stream
        .write_all(
            format!("GET {target} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .await
        .expect("request");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.expect("response");
    response
}

fn fixture(
    max_downloads: Option<u32>,
) -> (
    tempfile::TempDir,
    Arc<WebAccess>,
    quick_share_web::AccessToken,
    axum::Router,
) {
    let root = tempdir().expect("root");
    let file = root.path().join("payload.txt");
    fs::write(&file, b"payload").expect("file");
    let catalog = Arc::new(ShareCatalog::from_paths(&[file]).expect("catalog"));
    let (access, token) =
        WebAccess::generate(Duration::from_secs(30), max_downloads).expect("access");
    let access = Arc::new(access);
    let router = build_browser_router(
        catalog,
        Arc::clone(&access),
        None,
        Arc::new(ZipService::new(1, 2).expect("ZIP")),
    );
    (root, access, token, router)
}

#[tokio::test]
async fn explicit_http_serves_and_graceful_shutdown_releases_the_port() {
    let (_root, access, token, router) = fixture(None);
    let security = PreparedWebSecurity::prepare(
        WebSecurity::Http(ExplicitHttp::new(true).expect("explicit HTTP")),
        &[IpAddr::V4(Ipv4Addr::LOCALHOST)],
    )
    .await
    .expect("security");
    let cancellation = CancellationToken::new();
    let server = start_web_server(
        router,
        SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        security,
        token.expose(),
        Duration::from_secs(30),
        None,
        cancellation.clone(),
        access.quota_shutdown(),
    )
    .expect("server");
    let address = server.local_addr();
    assert!(server.endpoint.url.starts_with("http://127.0.0.1:"));
    let response = http_get(address, &format!("/api/catalog?token={}", token.expose())).await;
    assert!(response.starts_with(b"HTTP/1.1 200"));
    cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(6), server.wait())
        .await
        .expect("shutdown timeout")
        .expect("server shutdown");
    std::net::TcpListener::bind(address).expect("released port");
}

#[tokio::test]
async fn timeout_and_download_quota_each_stop_accepting_and_release_the_listener() {
    let (_root, access, token, router) = fixture(None);
    let security = PreparedWebSecurity::prepare(
        WebSecurity::Http(ExplicitHttp::new(true).expect("HTTP")),
        &[IpAddr::V4(Ipv4Addr::LOCALHOST)],
    )
    .await
    .expect("security");
    let server = start_web_server(
        router,
        SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        security,
        token.expose(),
        Duration::from_millis(50),
        None,
        CancellationToken::new(),
        access.quota_shutdown(),
    )
    .expect("server");
    let timeout_address = server.local_addr();
    tokio::time::timeout(Duration::from_secs(6), server.wait())
        .await
        .expect("timeout shutdown")
        .expect("server");
    std::net::TcpListener::bind(timeout_address).expect("timeout released port");

    let (_root, access, token, router) = fixture(Some(1));
    let listing = router.clone();
    let security = PreparedWebSecurity::prepare(
        WebSecurity::Http(ExplicitHttp::new(true).expect("HTTP")),
        &[IpAddr::V4(Ipv4Addr::LOCALHOST)],
    )
    .await
    .expect("security");
    let server = start_web_server(
        listing,
        SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        security,
        token.expose(),
        Duration::from_secs(30),
        Some(1),
        CancellationToken::new(),
        access.quota_shutdown(),
    )
    .expect("server");
    let quota_address = server.local_addr();
    let listing_response = http_get(
        quota_address,
        &format!("/api/catalog?token={}", token.expose()),
    )
    .await;
    let body = String::from_utf8_lossy(&listing_response);
    let marker = "\"kind\":\"file\"";
    assert!(body.contains(marker));
    let id_start = body.find("\"id\":\"").expect("ID") + 6;
    let id = &body[id_start..id_start + 36];
    let download = http_get(
        quota_address,
        &format!("/api/download/{id}?token={}", token.expose()),
    )
    .await;
    assert!(download.starts_with(b"HTTP/1.1 200"));
    tokio::time::timeout(Duration::from_secs(6), server.wait())
        .await
        .expect("quota shutdown")
        .expect("server");
    std::net::TcpListener::bind(quota_address).expect("quota released port");
}

#[tokio::test]
async fn default_https_listener_rejects_plaintext_http() {
    let (_root, access, token, router) = fixture(None);
    let security =
        PreparedWebSecurity::prepare(WebSecurity::SelfSigned, &[IpAddr::V4(Ipv4Addr::LOCALHOST)])
            .await
            .expect("TLS");
    let cancellation = CancellationToken::new();
    let server = start_web_server(
        router,
        SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        security,
        token.expose(),
        Duration::from_secs(30),
        None,
        cancellation.clone(),
        access.quota_shutdown(),
    )
    .expect("server");
    assert!(server.endpoint.url.starts_with("https://"));
    let response =
        tokio::time::timeout(Duration::from_secs(2), http_get(server.local_addr(), "/")).await;
    if let Ok(bytes) = response {
        assert!(!bytes.starts_with(b"HTTP/1.1 200"));
    }
    cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(6), server.wait())
        .await
        .expect("shutdown")
        .expect("server");
}
