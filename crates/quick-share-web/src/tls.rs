use axum_server::tls_rustls::RustlsConfig;
use qrcode::{QrCode, render::unicode};
use rcgen::{CertificateParams, KeyPair, SanType};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fmt,
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    time::Duration,
};
use thiserror::Error;
use zeroize::Zeroizing;

const MAX_CERTIFICATE_PEM_BYTES: u64 = 1024 * 1024;
const MAX_PRIVATE_KEY_PEM_BYTES: u64 = 256 * 1024;

#[derive(Clone)]
pub struct ExplicitHttp(());

impl ExplicitHttp {
    pub fn new(allow_http: bool) -> Result<Self, SecurityError> {
        if allow_http {
            Ok(Self(()))
        } else {
            Err(SecurityError::HttpRequiresExplicitOptIn)
        }
    }
}

impl fmt::Debug for ExplicitHttp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ExplicitHttp(explicit=true)")
    }
}

#[derive(Clone)]
pub enum WebSecurity {
    SelfSigned,
    UserProvided {
        certificate: PathBuf,
        private_key: PathBuf,
    },
    Http(ExplicitHttp),
}

impl fmt::Debug for WebSecurity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SelfSigned => formatter.write_str("WebSecurity::SelfSigned"),
            Self::UserProvided { .. } => formatter
                .debug_struct("WebSecurity::UserProvided")
                .field("certificate", &"[REDACTED]")
                .field("private_key", &"[REDACTED]")
                .finish(),
            Self::Http(explicit) => formatter
                .debug_tuple("WebSecurity::Http")
                .field(explicit)
                .finish(),
        }
    }
}

#[derive(Clone)]
pub struct PreparedWebSecurity {
    kind: PreparedKind,
}

#[derive(Clone)]
enum PreparedKind {
    Https {
        config: RustlsConfig,
        fingerprint: String,
        self_signed: bool,
    },
    Http,
}

impl PreparedWebSecurity {
    pub async fn prepare(
        security: WebSecurity,
        advertise_ips: &[IpAddr],
    ) -> Result<Self, SecurityError> {
        install_crypto_provider();
        match security {
            WebSecurity::SelfSigned => Self::self_signed(advertise_ips).await,
            WebSecurity::UserProvided {
                certificate,
                private_key,
            } => Self::user_provided(certificate, private_key).await,
            WebSecurity::Http(_) => Ok(Self {
                kind: PreparedKind::Http,
            }),
        }
    }

    async fn self_signed(advertise_ips: &[IpAddr]) -> Result<Self, SecurityError> {
        if advertise_ips.is_empty() {
            return Err(SecurityError::NoAdvertiseAddress);
        }
        let mut params =
            CertificateParams::new(vec!["localhost".to_owned(), "quick-share.local".to_owned()])?;
        let mut unique = BTreeSet::new();
        for ip in advertise_ips {
            if unique.insert(*ip) {
                params.subject_alt_names.push(SanType::IpAddress(*ip));
            }
        }
        let key = KeyPair::generate()?;
        let certificate = params.self_signed(&key)?;
        let fingerprint = hex::encode(Sha256::digest(certificate.der().as_ref()));
        let certificate_pem = certificate.pem().into_bytes();
        let private_key_pem = Zeroizing::new(key.serialize_pem().into_bytes());
        let config = RustlsConfig::from_pem(certificate_pem, private_key_pem.to_vec()).await?;
        Ok(Self {
            kind: PreparedKind::Https {
                config,
                fingerprint,
                self_signed: true,
            },
        })
    }

    async fn user_provided(
        certificate: PathBuf,
        private_key: PathBuf,
    ) -> Result<Self, SecurityError> {
        let certificate_pem = read_bounded(&certificate, MAX_CERTIFICATE_PEM_BYTES).await?;
        let private_key_pem =
            Zeroizing::new(read_bounded(&private_key, MAX_PRIVATE_KEY_PEM_BYTES).await?);
        let fingerprint = first_certificate_fingerprint(&certificate_pem)?;
        let config = RustlsConfig::from_pem(certificate_pem, private_key_pem.to_vec()).await?;
        Ok(Self {
            kind: PreparedKind::Https {
                config,
                fingerprint,
                self_signed: false,
            },
        })
    }

    #[must_use]
    pub fn scheme(&self) -> &'static str {
        match self.kind {
            PreparedKind::Https { .. } => "https",
            PreparedKind::Http => "http",
        }
    }

    #[must_use]
    pub fn is_self_signed(&self) -> bool {
        matches!(
            self.kind,
            PreparedKind::Https {
                self_signed: true,
                ..
            }
        )
    }

    #[must_use]
    pub fn certificate_fingerprint(&self) -> Option<&str> {
        match &self.kind {
            PreparedKind::Https { fingerprint, .. } => Some(fingerprint),
            PreparedKind::Http => None,
        }
    }

    pub(crate) fn rustls_config(&self) -> Option<RustlsConfig> {
        match &self.kind {
            PreparedKind::Https { config, .. } => Some(config.clone()),
            PreparedKind::Http => None,
        }
    }
}

impl fmt::Debug for PreparedWebSecurity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedWebSecurity")
            .field("scheme", &self.scheme())
            .field("self_signed", &self.is_self_signed())
            .field(
                "certificate_fingerprint",
                &self.certificate_fingerprint().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

pub struct EndpointInfo {
    pub url: String,
    pub qr: String,
    pub curl: String,
    pub wget: String,
    pub expires_in: Duration,
    pub max_downloads: Option<u32>,
    pub self_signed: bool,
    pub certificate_fingerprint: Option<String>,
}

impl fmt::Debug for EndpointInfo {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EndpointInfo")
            .field("url", &"[REDACTED_TOKEN_URL]")
            .field("qr", &"[REDACTED_TOKEN_QR]")
            .field("curl", &"[REDACTED_TOKEN_COMMAND]")
            .field("wget", &"[REDACTED_TOKEN_COMMAND]")
            .field("expires_in", &self.expires_in)
            .field("max_downloads", &self.max_downloads)
            .field("self_signed", &self.self_signed)
            .field("certificate_fingerprint", &self.certificate_fingerprint)
            .finish()
    }
}

pub fn endpoint_info(
    listener: SocketAddr,
    advertise_ip: IpAddr,
    security: &PreparedWebSecurity,
    token: &str,
    expires_in: Duration,
    max_downloads: Option<u32>,
) -> Result<EndpointInfo, SecurityError> {
    if token.len() != 32 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(SecurityError::InvalidAccessToken);
    }
    let advertised = SocketAddr::new(advertise_ip, listener.port());
    let url = format!("{}://{advertised}/?token={token}", security.scheme());
    let qr = QrCode::new(url.as_bytes())?
        .render::<unicode::Dense1x2>()
        .quiet_zone(true)
        .build();
    let (curl_flag, wget_flag) = if security.is_self_signed() {
        ("--insecure ", "--no-check-certificate ")
    } else {
        ("", "")
    };
    Ok(EndpointInfo {
        curl: format!("curl {curl_flag}--location '{url}'"),
        wget: format!("wget {wget_flag}'{url}'"),
        url,
        qr,
        expires_in,
        max_downloads,
        self_signed: security.is_self_signed(),
        certificate_fingerprint: security.certificate_fingerprint().map(str::to_owned),
    })
}

async fn read_bounded(path: &PathBuf, limit: u64) -> Result<Vec<u8>, SecurityError> {
    let metadata = tokio::fs::metadata(path).await?;
    if metadata.len() == 0 || metadata.len() > limit {
        return Err(SecurityError::PemTooLarge);
    }
    Ok(tokio::fs::read(path).await?)
}

fn first_certificate_fingerprint(pem: &[u8]) -> Result<String, SecurityError> {
    use rustls::pki_types::{CertificateDer, pem::PemObject};
    let certificate = CertificateDer::pem_slice_iter(pem)
        .next()
        .transpose()
        .map_err(|_| SecurityError::InvalidCertificate)?
        .ok_or(SecurityError::InvalidCertificate)?;
    Ok(hex::encode(Sha256::digest(certificate.as_ref())))
}

fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

#[derive(Debug, Error)]
pub enum SecurityError {
    #[error("plaintext HTTP requires explicit --allow-http opt-in")]
    HttpRequiresExplicitOptIn,
    #[error("at least one advertised IP is required for a self-signed certificate")]
    NoAdvertiseAddress,
    #[error("Web access token has an invalid shape")]
    InvalidAccessToken,
    #[error("certificate or private-key PEM is empty or exceeds its hard bound")]
    PemTooLarge,
    #[error("user certificate PEM is invalid")]
    InvalidCertificate,
    #[error("self-signed certificate generation failed")]
    Certificate(#[from] rcgen::Error),
    #[error("TLS configuration failed")]
    Tls(#[from] std::io::Error),
    #[error("QR code generation failed")]
    Qr(#[from] qrcode::types::QrError),
}
