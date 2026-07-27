use crate::catalog::{CatalogEntryKind, OpenedCatalogSource, ShareCatalog};
use bytes::Bytes;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Read, Write},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use thiserror::Error;
use tokio::sync::{Semaphore, mpsc};

const ZIP_CHUNK_BYTES: usize = 64 * 1024;

pub struct ZipService {
    workers: Arc<Semaphore>,
    channel_capacity: usize,
    active: Arc<AtomicUsize>,
}

impl ZipService {
    pub fn new(max_workers: usize, channel_capacity: usize) -> Result<Self, ZipError> {
        if max_workers == 0 || max_workers > 16 || channel_capacity == 0 || channel_capacity > 64 {
            return Err(ZipError::InvalidLimits);
        }
        Ok(Self {
            workers: Arc::new(Semaphore::new(max_workers)),
            channel_capacity,
            active: Arc::new(AtomicUsize::new(0)),
        })
    }

    pub(crate) fn start(
        &self,
        catalog: Arc<ShareCatalog>,
    ) -> Result<mpsc::Receiver<Result<Bytes, io::Error>>, ZipError> {
        let permit = Arc::clone(&self.workers)
            .try_acquire_owned()
            .map_err(|_| ZipError::Busy)?;
        let (sender, receiver) = mpsc::channel(self.channel_capacity);
        let active = Arc::clone(&self.active);
        active.fetch_add(1, Ordering::SeqCst);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let _active = ActiveWorker(active);
            if let Err(error) = write_archive(&catalog, sender.clone()) {
                let _ = sender.blocking_send(Err(io::Error::other(error.to_string())));
            }
        });
        Ok(receiver)
    }

    #[must_use]
    pub fn active_workers(&self) -> usize {
        self.active.load(Ordering::SeqCst)
    }
}

impl std::fmt::Debug for ZipService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ZipService")
            .field("channel_capacity", &self.channel_capacity)
            .field("active_workers", &self.active_workers())
            .finish_non_exhaustive()
    }
}

struct ActiveWorker(Arc<AtomicUsize>);

impl Drop for ActiveWorker {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn write_archive(
    catalog: &ShareCatalog,
    sender: mpsc::Sender<Result<Bytes, io::Error>>,
) -> Result<(), ZipError> {
    let channel = ChannelWriter::new(sender);
    let mut archive = zip::ZipWriter::new_stream(channel);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .large_file(true);
    let mut archive_paths = BTreeMap::new();
    let mut seen_paths = BTreeSet::new();
    for entry in catalog.entries() {
        let parent = entry
            .parent_id()
            .and_then(|id| archive_paths.get(id))
            .map(String::as_str);
        let name = unique_archive_path(parent, entry.name(), &mut seen_paths);
        archive_paths.insert(entry.id().clone(), name.clone());
        match entry.kind() {
            CatalogEntryKind::Directory => {
                archive.add_directory(format!("{}/", name.trim_end_matches('/')), options)?;
            }
            CatalogEntryKind::File => {
                archive.start_file(name, options)?;
                let opened = catalog.open_file(entry.id())?;
                match opened.source {
                    OpenedCatalogSource::File(mut file) => {
                        let mut buffer = [0_u8; ZIP_CHUNK_BYTES];
                        loop {
                            let read = file.read(&mut buffer)?;
                            if read == 0 {
                                break;
                            }
                            archive.write_all(&buffer[..read])?;
                        }
                    }
                    OpenedCatalogSource::Memory(bytes) => archive.write_all(&bytes)?,
                }
            }
        }
    }
    let stream = archive.finish()?;
    let mut channel = stream.into_inner();
    channel.flush()?;
    Ok(())
}

fn unique_archive_path(
    parent: Option<&str>,
    source_name: &str,
    seen: &mut BTreeSet<String>,
) -> String {
    let component = safe_archive_component(source_name);
    for index in 0..=crate::catalog::MAX_CATALOG_ENTRIES {
        let candidate_component = if index == 0 {
            component.clone()
        } else {
            append_name_index(&component, index)
        };
        let candidate = parent.map_or_else(
            || candidate_component.clone(),
            |parent| format!("{parent}/{candidate_component}"),
        );
        if seen.insert(candidate.to_lowercase()) {
            return candidate;
        }
    }
    let fallback = format!("{component}-{}", uuid::Uuid::now_v7());
    parent.map_or(fallback.clone(), |parent| format!("{parent}/{fallback}"))
}

fn safe_archive_component(source: &str) -> String {
    let mut value = source
        .chars()
        .map(|character| {
            if character.is_control() || "<>:\"/\\|?*".contains(character) {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();
    value.truncate(value.trim_end_matches([' ', '.']).len());
    if value.is_empty() || matches!(value.as_str(), "." | "..") {
        value = "_".to_owned();
    }
    let stem = value
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
    {
        value.insert(0, '_');
    }
    value
}

fn append_name_index(name: &str, index: usize) -> String {
    match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => format!("{stem} ({index}).{extension}"),
        _ => format!("{name} ({index})"),
    }
}

struct ChannelWriter {
    sender: mpsc::Sender<Result<Bytes, io::Error>>,
    pending: Vec<u8>,
}

impl ChannelWriter {
    fn new(sender: mpsc::Sender<Result<Bytes, io::Error>>) -> Self {
        Self {
            sender,
            pending: Vec::with_capacity(ZIP_CHUNK_BYTES),
        }
    }

    fn send_pending(&mut self) -> io::Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let bytes = Bytes::from(std::mem::take(&mut self.pending));
        self.pending = Vec::with_capacity(ZIP_CHUNK_BYTES);
        self.sender
            .blocking_send(Ok(bytes))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "ZIP client disconnected"))
    }
}

impl Write for ChannelWriter {
    fn write(&mut self, mut bytes: &[u8]) -> io::Result<usize> {
        let original = bytes.len();
        while !bytes.is_empty() {
            let available = ZIP_CHUNK_BYTES - self.pending.len();
            let take = available.min(bytes.len());
            self.pending.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            if self.pending.len() == ZIP_CHUNK_BYTES {
                self.send_pending()?;
            }
        }
        Ok(original)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.send_pending()
    }
}

#[derive(Debug, Error)]
pub enum ZipError {
    #[error("ZIP worker limits are invalid")]
    InvalidLimits,
    #[error("all ZIP workers are busy")]
    Busy,
    #[error("ZIP archive generation failed")]
    Zip(#[from] zip::result::ZipError),
    #[error(transparent)]
    Catalog(#[from] crate::catalog::CatalogError),
    #[error("ZIP stream I/O failed")]
    Io(#[from] io::Error),
}
