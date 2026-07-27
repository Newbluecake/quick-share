use anyhow::{Context, Result};
use axum::{Router, response::Html, routing::get};
use axum_server::{Handle, tls_rustls::RustlsConfig};
use clap::Parser;
use qrcode::{QrCode, render::unicode};
use rcgen::{CertificateParams, KeyPair, SanType};
use sha2::{Digest, Sha256};
use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};

#[derive(Debug, Parser)]
#[command(about = "Disposable Quick Share self-signed browser HTTPS experiment")]
struct Args {
    #[arg(long, default_value = "0.0.0.0:54431")]
    bind: SocketAddr,
    #[arg(long, default_value_t = 180)]
    duration: u64,
    #[arg(long)]
    advertise_ip: Option<IpAddr>,
}

fn certificate(advertise_ip: IpAddr) -> Result<(Vec<u8>, Vec<u8>, String)> {
    let mut params =
        CertificateParams::new(vec!["localhost".to_owned(), "quick-share.local".to_owned()])
            .context("create certificate parameters")?;
    params
        .subject_alt_names
        .push(SanType::IpAddress(advertise_ip));
    let key = KeyPair::generate().context("generate temporary key")?;
    let cert = params.self_signed(&key).context("self-sign certificate")?;
    let fingerprint = hex::encode(Sha256::digest(cert.der().as_ref()));
    Ok((
        cert.pem().into_bytes(),
        key.serialize_pem().into_bytes(),
        fingerprint,
    ))
}

async fn page() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Quick Share TLS Spike</title>
<style>body{font:16px system-ui;max-width:42rem;margin:3rem auto;padding:1rem}code{word-break:break-all}</style>
<h1>Quick Share HTTPS spike reached</h1>
<p>This is a disposable self-signed certificate experiment.</p>
<p>If this page loaded, record the browser, operating system, warning screens, and whether a QR-code launch worked.</p>"#,
    )
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let advertise_ip = match args.advertise_ip {
        Some(ip) => ip,
        None => local_ip_address::local_ip().context("detect LAN IP; use --advertise-ip")?,
    };
    let url = format!("https://{advertise_ip}:{}/", args.bind.port());
    let (cert_pem, key_pem, fingerprint) = certificate(advertise_ip)?;
    let config = RustlsConfig::from_pem(cert_pem, key_pem)
        .await
        .context("build rustls configuration")?;
    let qr = QrCode::new(url.as_bytes())?
        .render::<unicode::Dense1x2>()
        .quiet_zone(false)
        .build();

    println!("SELF-SIGNED HTTPS EXPERIMENT");
    println!("URL: {url}");
    println!("SHA-256 certificate fingerprint: {fingerprint}");
    println!("Expected: the browser warns because the certificate is not publicly trusted.");
    println!("Do not install this temporary certificate as a trusted CA.");
    println!("\n{qr}\n");
    println!("Server exits after {} seconds.", args.duration);

    let app = Router::new().route("/", get(page));
    let handle = Handle::new();
    let shutdown_handle = handle.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(args.duration)).await;
        shutdown_handle.graceful_shutdown(Some(Duration::from_secs(2)));
    });

    axum_server::bind_rustls(args.bind, config)
        .handle(handle)
        .serve(app.into_make_service())
        .await
        .context("serve HTTPS experiment")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::certificate;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn generated_certificate_has_stable_sha256_fingerprint_shape() {
        // Arrange
        let ip = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));

        // Act
        let (_cert, _key, fingerprint) = certificate(ip).expect("generate certificate");

        // Assert
        assert_eq!(fingerprint.len(), 64);
        assert!(fingerprint.chars().all(|value| value.is_ascii_hexdigit()));
    }

    #[test]
    fn every_run_generates_a_distinct_temporary_identity() {
        // Arrange
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);

        // Act
        let (_, _, first) = certificate(ip).expect("first certificate");
        let (_, _, second) = certificate(ip).expect("second certificate");

        // Assert
        assert_ne!(first, second);
    }
}
