#![forbid(unsafe_code)]
//! Secure traditional browser sharing mode.

mod access;
mod catalog;
mod http;
mod server;
mod tls;
mod upload;
mod zip_stream;

pub use access::{AccessError, AccessToken, WebAccess};
pub use catalog::{
    CatalogEntry, CatalogEntryKind, CatalogError, CatalogId, MAX_CATALOG_DISPLAY_BYTES,
    MAX_CATALOG_ENTRIES, ShareCatalog,
};
pub use http::{
    WebProgressEvent, build_browser_router, build_browser_router_with_progress,
    build_read_only_router, build_web_router,
};
pub use server::{RunningWebServer, ServerError, start_web_server};
pub use tls::{
    EndpointInfo, ExplicitHttp, PreparedWebSecurity, SecurityError, WebSecurity, endpoint_info,
};
pub use upload::{MAX_UPLOAD_FILES, UploadConfig, UploadError, UploadProgress, UploadService};
pub use zip_stream::{ZipError, ZipService};
