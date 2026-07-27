use crate::{
    CHECKSUM_SIGNATURE_ASSET, CHECKSUMS_ASSET, GITHUB_OWNER, GITHUB_REPOSITORY,
    MAX_RELEASE_METADATA_BYTES, ReleaseAsset, ReleaseInfo, ReleaseProvider, UpdateError,
    asset_name_for_target, current_target,
};
use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::{Client, Url, redirect::Policy};
use semver::Version;
use serde::Deserialize;
use std::path::Path;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

const API_ORIGIN: &str = "https://api.github.com";
const MAX_REDIRECTS: usize = 5;

#[derive(Clone)]
pub struct GitHubReleaseProvider {
    client: Client,
}

impl GitHubReleaseProvider {
    pub fn new() -> Result<Self, UpdateError> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = Client::builder()
            .user_agent(concat!("quick-share/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(120))
            .redirect(Policy::custom(|attempt| {
                if attempt.previous().len() >= MAX_REDIRECTS {
                    return attempt.error("too many update redirects");
                }
                let Some(previous) = attempt.previous().last() else {
                    return attempt.error("update redirect has no origin");
                };
                if is_allowed_redirect(previous, attempt.url()) {
                    attempt.follow()
                } else {
                    attempt.error("update redirect left the fixed GitHub origins")
                }
            }))
            .build()?;
        Ok(Self { client })
    }

    async fn response_bytes(
        &self,
        response: reqwest::Response,
        limit: usize,
    ) -> Result<Vec<u8>, UpdateError> {
        if !response.status().is_success() {
            return Err(UpdateError::HttpStatus(response.status().as_u16()));
        }
        if response
            .content_length()
            .is_some_and(|size| size > limit as u64)
        {
            return Err(UpdateError::MetadataTooLarge);
        }
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if bytes.len().saturating_add(chunk.len()) > limit {
                return Err(UpdateError::MetadataTooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
}

#[async_trait]
impl ReleaseProvider for GitHubReleaseProvider {
    async fn release(&self, requested: Option<&Version>) -> Result<ReleaseInfo, UpdateError> {
        let endpoint = match requested {
            Some(version) => format!(
                "{API_ORIGIN}/repos/{GITHUB_OWNER}/{GITHUB_REPOSITORY}/releases/tags/v{version}"
            ),
            None => {
                format!("{API_ORIGIN}/repos/{GITHUB_OWNER}/{GITHUB_REPOSITORY}/releases/latest")
            }
        };
        let response = self.client.get(endpoint).send().await?;
        if response.url().scheme() != "https" || response.url().host_str() != Some("api.github.com")
        {
            return Err(UpdateError::UntrustedAssetUrl);
        }
        let bytes = self
            .response_bytes(response, MAX_RELEASE_METADATA_BYTES)
            .await?;
        let metadata: GitHubRelease = serde_json::from_slice(&bytes)
            .map_err(|error| UpdateError::InvalidMetadata(error.to_string()))?;
        if metadata.draft {
            return Err(UpdateError::InvalidMetadata(
                "draft releases cannot be installed".to_owned(),
            ));
        }
        let version_text = metadata.tag_name.strip_prefix('v').ok_or_else(|| {
            UpdateError::InvalidMetadata("release tag must start with v".to_owned())
        })?;
        let version = Version::parse(version_text)?;
        if metadata.tag_name != format!("v{version}") {
            return Err(UpdateError::InvalidMetadata(
                "release tag is not canonical semver".to_owned(),
            ));
        }
        if metadata.prerelease && requested.is_none() {
            return Err(UpdateError::InvalidMetadata(
                "latest release unexpectedly points to a prerelease".to_owned(),
            ));
        }
        let target = current_target().ok_or(UpdateError::UnsupportedTarget)?;
        let binary_name = asset_name_for_target(target);
        let binary = find_asset(&metadata.assets, &binary_name, &metadata.tag_name)?;
        let checksums = find_asset(&metadata.assets, CHECKSUMS_ASSET, &metadata.tag_name)?;
        let signature = find_asset(
            &metadata.assets,
            CHECKSUM_SIGNATURE_ASSET,
            &metadata.tag_name,
        )?;
        Ok(ReleaseInfo {
            version,
            tag: metadata.tag_name,
            binary,
            checksums,
            signature,
        })
    }

    async fn download(
        &self,
        asset: &ReleaseAsset,
        destination: &Path,
        limit: u64,
        cancellation: &CancellationToken,
    ) -> Result<(), UpdateError> {
        if asset.size > limit {
            return Err(UpdateError::AssetTooLarge(limit));
        }
        validate_asset_url(&asset.download_url, None)?;
        let response = tokio::select! {
            () = cancellation.cancelled() => return Err(UpdateError::Cancelled),
            response = self.client.get(&asset.download_url).send() => response?,
        };
        if !response.status().is_success() {
            return Err(UpdateError::HttpStatus(response.status().as_u16()));
        }
        if response.content_length().is_some_and(|size| size > limit) {
            return Err(UpdateError::AssetTooLarge(limit));
        }
        if !is_allowed_download_destination(response.url()) {
            return Err(UpdateError::UntrustedAssetUrl);
        }
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)
            .await?;
        let mut received = 0_u64;
        let mut stream = response.bytes_stream();
        loop {
            let next = tokio::select! {
                () = cancellation.cancelled() => return Err(UpdateError::Cancelled),
                next = stream.next() => next,
            };
            let Some(chunk) = next else {
                break;
            };
            let chunk = chunk?;
            received = received
                .checked_add(chunk.len() as u64)
                .ok_or(UpdateError::AssetTooLarge(limit))?;
            if received > limit {
                return Err(UpdateError::AssetTooLarge(limit));
            }
            file.write_all(&chunk).await?;
        }
        if received != asset.size {
            return Err(UpdateError::DownloadInterrupted);
        }
        file.flush().await?;
        file.sync_all().await?;
        Ok(())
    }
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

fn find_asset(assets: &[GitHubAsset], name: &str, tag: &str) -> Result<ReleaseAsset, UpdateError> {
    let mut matches = assets.iter().filter(|asset| asset.name == name);
    let asset = matches
        .next()
        .ok_or_else(|| UpdateError::MissingAsset(name.to_owned()))?;
    if matches.next().is_some() {
        return Err(UpdateError::InvalidMetadata(format!(
            "release contains duplicate asset {name}"
        )));
    }
    validate_asset_url(&asset.browser_download_url, Some(tag))?;
    Ok(ReleaseAsset {
        name: asset.name.clone(),
        download_url: asset.browser_download_url.clone(),
        size: asset.size,
    })
}

pub fn validate_asset_url(source: &str, expected_tag: Option<&str>) -> Result<(), UpdateError> {
    let url = Url::parse(source).map_err(|_| UpdateError::UntrustedAssetUrl)?;
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(UpdateError::UntrustedAssetUrl);
    }
    let prefix = format!("/{GITHUB_OWNER}/{GITHUB_REPOSITORY}/releases/download/");
    let Some(rest) = url.path().strip_prefix(&prefix) else {
        return Err(UpdateError::UntrustedAssetUrl);
    };
    let (tag, asset) = rest.split_once('/').ok_or(UpdateError::UntrustedAssetUrl)?;
    if tag.is_empty() || asset.is_empty() || asset.contains('/') {
        return Err(UpdateError::UntrustedAssetUrl);
    }
    if expected_tag.is_some_and(|expected| tag != expected) {
        return Err(UpdateError::UntrustedAssetUrl);
    }
    Ok(())
}

#[must_use]
pub fn is_allowed_redirect(previous: &Url, next: &Url) -> bool {
    is_allowed_download_destination(previous) && is_allowed_download_destination(next)
}

fn is_allowed_download_destination(url: &Url) -> bool {
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    matches!(
        url.host_str(),
        Some("github.com" | "api.github.com" | "release-assets.githubusercontent.com")
    )
}
