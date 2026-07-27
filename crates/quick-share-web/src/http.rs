use crate::{
    access::{AccessError, WebAccess},
    catalog::{CatalogError, CatalogId, OpenedCatalogFile, OpenedCatalogSource, ShareCatalog},
    upload::{CompletedUpload, UploadError, UploadService},
    zip_stream::{ZipError, ZipService},
};
use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Path, Query, State},
    http::{HeaderMap, HeaderValue, Request, StatusCode, header},
    middleware::{Next, from_fn},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use std::{io, sync::Arc};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncSeekExt, SeekFrom},
    sync::mpsc,
};

const CSP: &str = "default-src 'self'; base-uri 'none'; object-src 'none'; frame-ancestors 'none'; form-action 'self'; img-src 'self' data:; style-src 'self'; script-src 'self'; connect-src 'self'";

#[derive(Clone)]
struct WebState {
    catalog: Arc<ShareCatalog>,
    access: Arc<WebAccess>,
    upload: Option<Arc<UploadService>>,
    zip: Option<Arc<ZipService>>,
    progress: Option<mpsc::Sender<WebProgressEvent>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebProgressEvent {
    DownloadStarted {
        name: String,
        total_bytes: u64,
    },
    DownloadProgress {
        name: String,
        transferred_bytes: u64,
        total_bytes: u64,
    },
    DownloadCompleted {
        name: String,
        total_bytes: u64,
    },
    DownloadInterrupted {
        name: String,
        transferred_bytes: u64,
    },
    UploadCompleted {
        files: usize,
        total_bytes: u64,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TokenQuery {
    token: Option<String>,
}

pub fn build_read_only_router(catalog: Arc<ShareCatalog>, access: Arc<WebAccess>) -> Router {
    build_router(catalog, access, None, None, None)
}

pub fn build_web_router(
    catalog: Arc<ShareCatalog>,
    access: Arc<WebAccess>,
    upload: Option<Arc<UploadService>>,
) -> Router {
    build_router(catalog, access, upload, None, None)
}

pub fn build_browser_router(
    catalog: Arc<ShareCatalog>,
    access: Arc<WebAccess>,
    upload: Option<Arc<UploadService>>,
    zip: Arc<ZipService>,
) -> Router {
    build_router(catalog, access, upload, Some(zip), None)
}

pub fn build_browser_router_with_progress(
    catalog: Arc<ShareCatalog>,
    access: Arc<WebAccess>,
    upload: Option<Arc<UploadService>>,
    zip: Arc<ZipService>,
    progress: mpsc::Sender<WebProgressEvent>,
) -> Router {
    build_router(catalog, access, upload, Some(zip), Some(progress))
}

fn build_router(
    catalog: Arc<ShareCatalog>,
    access: Arc<WebAccess>,
    upload: Option<Arc<UploadService>>,
    zip: Option<Arc<ZipService>>,
    progress: Option<mpsc::Sender<WebProgressEvent>>,
) -> Router {
    let max_body_bytes = upload.as_ref().map(|service| service.max_body_bytes());
    let mut router = Router::new()
        .route("/", get(index_page))
        .route("/assets/app.css", get(stylesheet))
        .route("/assets/app.js", get(javascript))
        .route("/api/catalog", get(list_catalog))
        .route("/api/download/{id}", get(download_entry))
        .route("/api/preview/{id}", get(preview_entry));
    if upload.is_some() {
        router = router.route("/api/upload", post(upload_files));
    }
    if zip.is_some() {
        router = router.route("/api/download-all", get(download_all));
    }
    let router = router.with_state(WebState {
        catalog,
        access,
        upload,
        zip,
        progress,
    });
    let router = if let Some(limit) = max_body_bytes {
        router.layer(DefaultBodyLimit::max(limit))
    } else {
        router
    };
    router
        .layer(tower::limit::ConcurrencyLimitLayer::new(64))
        .layer(from_fn(security_headers))
}

const INDEX_HTML: &str = include_str!("../assets/index.html");
const APP_CSS: &str = include_str!("../assets/app.css");
const APP_JS: &str = include_str!("../assets/app.js");

async fn index_page(
    State(state): State<WebState>,
    Query(query): Query<TokenQuery>,
) -> Result<Html<&'static str>, WebError> {
    let token = query.token.as_deref().ok_or(AccessError::Unauthorized)?;
    state.access.authorize(token)?;
    Ok(Html(INDEX_HTML))
}

async fn stylesheet() -> Response {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], APP_CSS).into_response()
}

async fn javascript() -> Response {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        APP_JS,
    )
        .into_response()
}

async fn list_catalog(
    State(state): State<WebState>,
    Query(query): Query<TokenQuery>,
) -> Result<Json<CatalogResponse>, WebError> {
    let token = query.token.as_deref().ok_or(AccessError::Unauthorized)?;
    state.access.authorize(token)?;
    let entries = state
        .catalog
        .entries()
        .iter()
        .map(CatalogEntryResponse::from)
        .collect();
    Ok(Json(CatalogResponse {
        entries,
        upload_enabled: state.upload.is_some(),
        upload_password_required: state
            .upload
            .as_ref()
            .is_some_and(|upload| upload.requires_password()),
    }))
}

async fn download_entry(
    State(state): State<WebState>,
    Path(id): Path<String>,
    Query(query): Query<TokenQuery>,
    headers: HeaderMap,
) -> Result<Response, WebError> {
    let token = query.token.as_deref().ok_or(AccessError::Unauthorized)?;
    state.access.authorize(token)?;
    let id = CatalogId::parse(&id)?;
    let catalog = Arc::clone(&state.catalog);
    let opened = tokio::task::spawn_blocking(move || catalog.open_file(&id))
        .await
        .map_err(|_| WebError::Internal)??;
    let range = parse_range(headers.get(header::RANGE), opened.len)?;
    state.access.begin_download(token)?;
    stream_file(opened, range, false, state.progress).await
}

async fn preview_entry(
    State(state): State<WebState>,
    Path(id): Path<String>,
    Query(query): Query<TokenQuery>,
    headers: HeaderMap,
) -> Result<Response, WebError> {
    let token = query.token.as_deref().ok_or(AccessError::Unauthorized)?;
    state.access.authorize(token)?;
    let id = CatalogId::parse(&id)?;
    let catalog = Arc::clone(&state.catalog);
    let opened = tokio::task::spawn_blocking(move || catalog.open_file(&id))
        .await
        .map_err(|_| WebError::Internal)??;
    let media_type = mime_guess::from_path(&opened.name).first_or_octet_stream();
    if !matches!(
        media_type.essence_str(),
        "text/plain"
            | "text/csv"
            | "application/json"
            | "image/png"
            | "image/jpeg"
            | "image/gif"
            | "image/webp"
            | "image/bmp"
    ) {
        return Err(WebError::PreviewUnsupported);
    }
    let range = parse_range(headers.get(header::RANGE), opened.len)?;
    state.access.begin_download(token)?;
    stream_file(opened, range, true, state.progress).await
}

async fn download_all(
    State(state): State<WebState>,
    Query(query): Query<TokenQuery>,
) -> Result<Response, WebError> {
    let token = query.token.as_deref().ok_or(AccessError::Unauthorized)?;
    state.access.authorize(token)?;
    let zip = state.zip.as_ref().ok_or(WebError::NotFound)?;
    let receiver = zip.start(Arc::clone(&state.catalog))?;
    state.access.begin_download(token)?;
    let body = forward_zip(receiver, state.progress);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/zip")
        .header(
            header::CONTENT_DISPOSITION,
            "attachment; filename=\"quick-share.zip\"",
        )
        .body(body)
        .map_err(|_| WebError::Internal)
}

async fn upload_files(
    State(state): State<WebState>,
    Query(query): Query<TokenQuery>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<UploadResponse>), WebError> {
    let token = query.token.as_deref().ok_or(AccessError::Unauthorized)?;
    state.access.authorize(token)?;
    let upload = state.upload.as_ref().ok_or(WebError::NotFound)?;
    let _permit = upload.begin_request().await?;
    let password = headers
        .get("x-quick-share-upload-password")
        .map(|value| value.to_str().map_err(|_| UploadError::Unauthorized))
        .transpose()?;
    upload.authorize_password(password)?;
    let mut prepared = Vec::new();
    let mut request_bytes = 0_u64;
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(UploadError::Multipart)?
    {
        let Some(file_name) = field.file_name().map(str::to_owned) else {
            continue;
        };
        if prepared.len() >= crate::upload::MAX_UPLOAD_FILES {
            return Err(UploadError::TotalTooLarge.into());
        }
        prepared.push(
            upload
                .stage_field(&file_name, &mut field, &mut request_bytes)
                .await?,
        );
    }
    if prepared.is_empty() {
        return Err(UploadError::InvalidFileName.into());
    }
    let mut completed = Vec::with_capacity(prepared.len());
    for item in prepared {
        completed.push(upload.commit(item).await?);
    }
    if let Some(progress) = &state.progress {
        let _ = progress.try_send(WebProgressEvent::UploadCompleted {
            files: completed.len(),
            total_bytes: request_bytes,
        });
    }
    Ok((
        StatusCode::CREATED,
        Json(UploadResponse {
            files: completed
                .into_iter()
                .map(UploadFileResponse::from)
                .collect(),
            total_bytes: request_bytes,
        }),
    ))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UploadResponse {
    files: Vec<UploadFileResponse>,
    total_bytes: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UploadFileResponse {
    file_name: Option<String>,
    bytes: u64,
    skipped: bool,
}

impl From<CompletedUpload> for UploadFileResponse {
    fn from(value: CompletedUpload) -> Self {
        Self {
            file_name: value.file_name,
            bytes: value.bytes,
            skipped: value.skipped,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CatalogResponse {
    entries: Vec<CatalogEntryResponse>,
    upload_enabled: bool,
    upload_password_required: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CatalogEntryResponse {
    id: String,
    parent_id: Option<String>,
    name: String,
    display_path: String,
    kind: crate::catalog::CatalogEntryKind,
    size: u64,
    media_type: Option<String>,
}

impl From<&crate::catalog::CatalogEntry> for CatalogEntryResponse {
    fn from(entry: &crate::catalog::CatalogEntry) -> Self {
        Self {
            id: entry.id().to_string(),
            parent_id: entry.parent_id().map(ToString::to_string),
            name: entry.name().to_owned(),
            display_path: entry.display_path().to_owned(),
            kind: entry.kind(),
            size: entry.size(),
            media_type: entry.media_type().map(str::to_owned),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ByteRange {
    start: u64,
    end: u64,
}

impl ByteRange {
    fn len(self) -> u64 {
        self.end - self.start + 1
    }
}

fn parse_range(value: Option<&HeaderValue>, len: u64) -> Result<Option<ByteRange>, WebError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.to_str().map_err(|_| WebError::InvalidRange)?;
    let value = value.strip_prefix("bytes=").ok_or(WebError::InvalidRange)?;
    if value.contains(',') || value.is_empty() || len == 0 {
        return Err(WebError::InvalidRange);
    }
    let (start, end) = value.split_once('-').ok_or(WebError::InvalidRange)?;
    let range = if start.is_empty() {
        let suffix = end.parse::<u64>().map_err(|_| WebError::InvalidRange)?;
        if suffix == 0 {
            return Err(WebError::InvalidRange);
        }
        let count = suffix.min(len);
        ByteRange {
            start: len - count,
            end: len - 1,
        }
    } else {
        let start = start.parse::<u64>().map_err(|_| WebError::InvalidRange)?;
        if start >= len {
            return Err(WebError::InvalidRange);
        }
        let end = if end.is_empty() {
            len - 1
        } else {
            end.parse::<u64>()
                .map_err(|_| WebError::InvalidRange)?
                .min(len - 1)
        };
        if end < start {
            return Err(WebError::InvalidRange);
        }
        ByteRange { start, end }
    };
    Ok(Some(range))
}

async fn stream_file(
    opened: OpenedCatalogFile,
    range: Option<ByteRange>,
    inline: bool,
    progress: Option<mpsc::Sender<WebProgressEvent>>,
) -> Result<Response, WebError> {
    let (status, start, end, content_length) = match range {
        Some(range) => (
            StatusCode::PARTIAL_CONTENT,
            range.start,
            range.end,
            range.len(),
        ),
        None => (StatusCode::OK, 0, opened.len.saturating_sub(1), opened.len),
    };
    let name = opened.name.clone();
    let body = match opened.source {
        OpenedCatalogSource::File(file) => {
            let mut file = tokio::fs::File::from_std(file);
            if start != 0 {
                file.seek(SeekFrom::Start(start)).await?;
            }
            progress_body(
                file.take(content_length),
                name.clone(),
                content_length,
                progress,
            )
        }
        OpenedCatalogSource::Memory(bytes) => {
            let start = usize::try_from(start).map_err(|_| WebError::Internal)?;
            let length = usize::try_from(content_length).map_err(|_| WebError::Internal)?;
            let end = start.checked_add(length).ok_or(WebError::Internal)?;
            let contents =
                bytes::Bytes::copy_from_slice(bytes.get(start..end).ok_or(WebError::Internal)?);
            progress_body(
                std::io::Cursor::new(contents),
                name.clone(),
                content_length,
                progress,
            )
        }
    };
    let content_type = mime_guess::from_path(&opened.name).first_or_octet_stream();
    let disposition = content_disposition(&opened.name, inline);
    let mut builder = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type.as_ref())
        .header(header::CONTENT_LENGTH, content_length)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_DISPOSITION, disposition);
    if range.is_some() {
        builder = builder.header(
            header::CONTENT_RANGE,
            format!("bytes {start}-{end}/{}", opened.len),
        );
    }
    builder.body(body).map_err(|_| WebError::Internal)
}

fn progress_body<R>(
    mut reader: R,
    name: String,
    total_bytes: u64,
    progress: Option<mpsc::Sender<WebProgressEvent>>,
) -> Body
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let (body_sender, receiver) = mpsc::channel::<Result<bytes::Bytes, io::Error>>(4);
    tokio::spawn(async move {
        emit_progress(
            &progress,
            WebProgressEvent::DownloadStarted {
                name: name.clone(),
                total_bytes,
            },
        );
        let mut transferred = 0_u64;
        let mut last_reported = 0_u64;
        let mut buffer = vec![0_u8; 64 * 1024];
        loop {
            match reader.read(&mut buffer).await {
                Ok(0) => {
                    emit_progress(
                        &progress,
                        WebProgressEvent::DownloadCompleted {
                            name,
                            total_bytes: transferred,
                        },
                    );
                    break;
                }
                Ok(count) => {
                    transferred = transferred.saturating_add(count as u64);
                    if body_sender
                        .send(Ok(bytes::Bytes::copy_from_slice(&buffer[..count])))
                        .await
                        .is_err()
                    {
                        emit_progress(
                            &progress,
                            WebProgressEvent::DownloadInterrupted {
                                name,
                                transferred_bytes: transferred,
                            },
                        );
                        break;
                    }
                    if transferred == total_bytes
                        || transferred.saturating_sub(last_reported) >= 1024 * 1024
                    {
                        last_reported = transferred;
                        emit_progress(
                            &progress,
                            WebProgressEvent::DownloadProgress {
                                name: name.clone(),
                                transferred_bytes: transferred,
                                total_bytes,
                            },
                        );
                    }
                }
                Err(error) => {
                    let _ = body_sender.send(Err(error)).await;
                    emit_progress(
                        &progress,
                        WebProgressEvent::DownloadInterrupted {
                            name,
                            transferred_bytes: transferred,
                        },
                    );
                    break;
                }
            }
        }
    });
    let stream = futures_util::stream::unfold(receiver, |mut receiver| async move {
        receiver.recv().await.map(|item| (item, receiver))
    });
    Body::from_stream(stream)
}

fn forward_zip(
    mut source: mpsc::Receiver<Result<bytes::Bytes, io::Error>>,
    progress: Option<mpsc::Sender<WebProgressEvent>>,
) -> Body {
    let (body_sender, receiver) = mpsc::channel::<Result<bytes::Bytes, io::Error>>(4);
    tokio::spawn(async move {
        let name = "quick-share.zip".to_owned();
        emit_progress(
            &progress,
            WebProgressEvent::DownloadStarted {
                name: name.clone(),
                total_bytes: 0,
            },
        );
        let mut transferred = 0_u64;
        let mut last_reported = 0_u64;
        while let Some(item) = source.recv().await {
            match item {
                Ok(bytes) => {
                    transferred = transferred.saturating_add(bytes.len() as u64);
                    if body_sender.send(Ok(bytes)).await.is_err() {
                        emit_progress(
                            &progress,
                            WebProgressEvent::DownloadInterrupted {
                                name,
                                transferred_bytes: transferred,
                            },
                        );
                        return;
                    }
                    if transferred.saturating_sub(last_reported) >= 1024 * 1024 {
                        last_reported = transferred;
                        emit_progress(
                            &progress,
                            WebProgressEvent::DownloadProgress {
                                name: name.clone(),
                                transferred_bytes: transferred,
                                total_bytes: 0,
                            },
                        );
                    }
                }
                Err(error) => {
                    let _ = body_sender.send(Err(error)).await;
                    emit_progress(
                        &progress,
                        WebProgressEvent::DownloadInterrupted {
                            name,
                            transferred_bytes: transferred,
                        },
                    );
                    return;
                }
            }
        }
        emit_progress(
            &progress,
            WebProgressEvent::DownloadCompleted {
                name,
                total_bytes: transferred,
            },
        );
    });
    let stream = futures_util::stream::unfold(receiver, |mut receiver| async move {
        receiver.recv().await.map(|item| (item, receiver))
    });
    Body::from_stream(stream)
}

fn emit_progress(progress: &Option<mpsc::Sender<WebProgressEvent>>, event: WebProgressEvent) {
    if let Some(progress) = progress {
        let _ = progress.try_send(event);
    }
}

fn content_disposition(name: &str, inline: bool) -> String {
    let fallback = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    let fallback = if fallback.is_empty() {
        "download"
    } else {
        &fallback
    };
    let encoded = utf8_percent_encode(name, NON_ALPHANUMERIC);
    let mode = if inline { "inline" } else { "attachment" };
    format!("{mode}; filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
}

async fn security_headers(request: Request<Body>, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    response
}

#[derive(Debug, thiserror::Error)]
enum WebError {
    #[error(transparent)]
    Access(#[from] AccessError),
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    #[error(transparent)]
    Upload(#[from] UploadError),
    #[error(transparent)]
    Zip(#[from] ZipError),
    #[error("invalid byte range")]
    InvalidRange,
    #[error("preview media type is not allowed")]
    PreviewUnsupported,
    #[error("streaming file I/O failed")]
    Io(#[from] io::Error),
    #[error("Web route was not found")]
    NotFound,
    #[error("Web service internal error")]
    Internal,
}

#[derive(Serialize)]
struct ErrorBody {
    error: &'static str,
}

impl IntoResponse for WebError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::Access(AccessError::Unauthorized) | Self::Upload(UploadError::Unauthorized) => {
                (StatusCode::UNAUTHORIZED, "unauthorized")
            }
            Self::Access(AccessError::QuotaExceeded) => {
                (StatusCode::TOO_MANY_REQUESTS, "download limit reached")
            }
            Self::Upload(UploadError::RateLimited | UploadError::Busy) => {
                (StatusCode::TOO_MANY_REQUESTS, "upload limit reached")
            }
            Self::Zip(ZipError::Busy) => {
                (StatusCode::TOO_MANY_REQUESTS, "ZIP worker limit reached")
            }
            Self::Catalog(CatalogError::InvalidId) => {
                (StatusCode::BAD_REQUEST, "invalid catalog entry")
            }
            Self::Catalog(CatalogError::NotFound | CatalogError::NotAFile) => {
                (StatusCode::NOT_FOUND, "catalog entry not found")
            }
            Self::Catalog(CatalogError::SourceChanged | CatalogError::Io(_)) => {
                (StatusCode::CONFLICT, "catalog source changed")
            }
            Self::Upload(UploadError::Multipart(error))
                if error.status() == StatusCode::PAYLOAD_TOO_LARGE =>
            {
                (StatusCode::PAYLOAD_TOO_LARGE, "upload is too large")
            }
            Self::Upload(UploadError::InvalidFileName | UploadError::Multipart(_)) => {
                (StatusCode::BAD_REQUEST, "invalid upload")
            }
            Self::Upload(UploadError::FileTooLarge | UploadError::TotalTooLarge) => {
                (StatusCode::PAYLOAD_TOO_LARGE, "upload is too large")
            }
            Self::Upload(UploadError::Path(
                quick_share_core::paths::PathError::DestinationExists(_)
                | quick_share_core::paths::PathError::InteractionRequired(_),
            )) => (StatusCode::CONFLICT, "upload destination conflict"),
            Self::Upload(UploadError::Storage(_) | UploadError::Io(_)) => (
                StatusCode::INSUFFICIENT_STORAGE,
                "upload could not be stored",
            ),
            Self::InvalidRange => (StatusCode::RANGE_NOT_SATISFIABLE, "invalid byte range"),
            Self::PreviewUnsupported => (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "preview is not available",
            ),
            Self::NotFound => (StatusCode::NOT_FOUND, "not found"),
            Self::Access(
                AccessError::InvalidLimit | AccessError::EntropyUnavailable | AccessError::Internal,
            )
            | Self::Catalog(_)
            | Self::Upload(_)
            | Self::Zip(_)
            | Self::Io(_)
            | Self::Internal => (StatusCode::INTERNAL_SERVER_ERROR, "internal error"),
        };
        (status, Json(ErrorBody { error: message })).into_response()
    }
}
