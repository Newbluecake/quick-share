use quick_share_core::paths::RelativePath;
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};
use thiserror::Error;
use uuid::Uuid;

pub const MAX_CATALOG_ENTRIES: usize = 10_000;
pub const MAX_CATALOG_DISPLAY_BYTES: usize = 8 * 1024 * 1024;
const MAX_CATALOG_DEPTH: usize = 64;
const MAX_DISPLAY_PATH_BYTES: usize = 4 * 1024;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct CatalogId(String);

impl CatalogId {
    fn generate() -> Self {
        Self(Uuid::now_v7().to_string())
    }

    pub fn parse(source: &str) -> Result<Self, CatalogError> {
        if source.len() != 36 || source.bytes().any(|byte| byte.is_ascii_uppercase()) {
            return Err(CatalogError::InvalidId);
        }
        let parsed = Uuid::parse_str(source).map_err(|_| CatalogError::InvalidId)?;
        if parsed.to_string() != source {
            return Err(CatalogError::InvalidId);
        }
        Ok(Self(source.to_owned()))
    }
}

impl fmt::Display for CatalogId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl fmt::Debug for CatalogId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("CatalogId").field(&self.0).finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogEntryKind {
    File,
    Directory,
}

#[derive(Clone)]
pub struct CatalogEntry {
    id: CatalogId,
    parent_id: Option<CatalogId>,
    name: String,
    display_path: String,
    kind: CatalogEntryKind,
    size: u64,
    media_type: Option<String>,
    source_path: PathBuf,
    canonical_path: PathBuf,
    canonical_root: PathBuf,
    memory: Option<std::sync::Arc<[u8]>>,
}

impl CatalogEntry {
    #[must_use]
    pub fn id(&self) -> &CatalogId {
        &self.id
    }

    #[must_use]
    pub fn display_path(&self) -> &str {
        &self.display_path
    }

    #[must_use]
    pub fn kind(&self) -> CatalogEntryKind {
        self.kind
    }

    pub(crate) fn parent_id(&self) -> Option<&CatalogId> {
        self.parent_id.as_ref()
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn media_type(&self) -> Option<&str> {
        self.media_type.as_deref()
    }

    #[must_use]
    pub const fn size(&self) -> u64 {
        self.size
    }
}

impl fmt::Debug for CatalogEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CatalogEntry")
            .field("id", &self.id)
            .field("parent_id", &self.parent_id)
            .field("name", &self.name)
            .field("display_path", &self.display_path)
            .field("kind", &self.kind)
            .field("size", &self.size)
            .field("media_type", &self.media_type)
            .field("source_path", &"[REDACTED]")
            .field("canonical_path", &"[REDACTED]")
            .field("canonical_root", &"[REDACTED]")
            .field(
                "memory",
                &self
                    .memory
                    .as_ref()
                    .map(|bytes| format!("[REDACTED {} bytes]", bytes.len())),
            )
            .finish()
    }
}

#[derive(Clone)]
pub struct ShareCatalog {
    entries: Vec<CatalogEntry>,
    by_id: BTreeMap<CatalogId, usize>,
}

impl ShareCatalog {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            entries: Vec::new(),
            by_id: BTreeMap::new(),
        }
    }

    pub fn from_text(name: &str, bytes: std::sync::Arc<[u8]>) -> Result<Self, CatalogError> {
        if name.is_empty()
            || name.contains('/')
            || name.contains('\\')
            || RelativePath::parse(name).is_err()
        {
            return Err(CatalogError::InvalidMemoryName);
        }
        let id = CatalogId::generate();
        let entry = CatalogEntry {
            id: id.clone(),
            parent_id: None,
            name: safe_display(name),
            display_path: safe_display(name),
            kind: CatalogEntryKind::File,
            size: bytes.len() as u64,
            media_type: Some("text/plain; charset=utf-8".to_owned()),
            source_path: PathBuf::new(),
            canonical_path: PathBuf::new(),
            canonical_root: PathBuf::new(),
            memory: Some(bytes),
        };
        Ok(Self {
            entries: vec![entry],
            by_id: BTreeMap::from([(id, 0)]),
        })
    }

    pub fn from_paths(sources: &[PathBuf]) -> Result<Self, CatalogError> {
        if sources.is_empty() {
            return Err(CatalogError::NoSources);
        }
        let mut builder = CatalogBuilder {
            entries: Vec::new(),
            top_names: BTreeSet::new(),
            display_bytes: 0,
        };
        for source in sources {
            builder.add_root(source)?;
        }
        let by_id = builder
            .entries
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.id.clone(), index))
            .collect();
        Ok(Self {
            entries: builder.entries,
            by_id,
        })
    }

    #[must_use]
    pub fn entries(&self) -> &[CatalogEntry] {
        &self.entries
    }

    pub(crate) fn open_file(&self, id: &CatalogId) -> Result<OpenedCatalogFile, CatalogError> {
        let entry = self
            .by_id
            .get(id)
            .and_then(|index| self.entries.get(*index))
            .ok_or(CatalogError::NotFound)?;
        if entry.kind != CatalogEntryKind::File {
            return Err(CatalogError::NotAFile);
        }
        if let Some(bytes) = &entry.memory {
            return Ok(OpenedCatalogFile {
                source: OpenedCatalogSource::Memory(Arc::clone(bytes)),
                len: bytes.len() as u64,
                name: entry.name.clone(),
            });
        }
        verify_entry(entry)?;
        let file = fs::File::open(&entry.source_path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(CatalogError::SourceChanged);
        }
        verify_entry(entry)?;
        Ok(OpenedCatalogFile {
            source: OpenedCatalogSource::File(file),
            len: metadata.len(),
            name: entry.name.clone(),
        })
    }
}

impl fmt::Debug for ShareCatalog {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ShareCatalog")
            .field("entries", &self.entries)
            .finish()
    }
}

pub(crate) struct OpenedCatalogFile {
    pub source: OpenedCatalogSource,
    pub len: u64,
    pub name: String,
}

pub(crate) enum OpenedCatalogSource {
    File(fs::File),
    Memory(std::sync::Arc<[u8]>),
}

struct CatalogBuilder {
    entries: Vec<CatalogEntry>,
    top_names: BTreeSet<String>,
    display_bytes: usize,
}

impl CatalogBuilder {
    fn add_root(&mut self, source: &Path) -> Result<(), CatalogError> {
        let metadata = fs::symlink_metadata(source)?;
        if metadata.file_type().is_symlink() {
            return Err(CatalogError::SymlinkRoot);
        }
        if !metadata.is_file() && !metadata.is_dir() {
            return Err(CatalogError::UnsupportedSource);
        }
        let canonical = fs::canonicalize(source)?;
        let raw_name = source
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .ok_or(CatalogError::NonUtf8Path)?;
        let name = unique_top_name(&safe_display(raw_name), &mut self.top_names);
        let display_name = name.clone();
        let root_id = self.push_entry(
            None,
            name.clone(),
            name,
            if metadata.is_dir() {
                CatalogEntryKind::Directory
            } else {
                CatalogEntryKind::File
            },
            if metadata.is_file() {
                metadata.len()
            } else {
                0
            },
            source.to_path_buf(),
            canonical.clone(),
            canonical.clone(),
        )?;
        if metadata.is_dir() {
            self.walk_directory(source, &canonical, &root_id, display_name, 1)?;
        }
        Ok(())
    }

    fn walk_directory(
        &mut self,
        directory: &Path,
        canonical_root: &Path,
        parent_id: &CatalogId,
        parent_display: String,
        depth: usize,
    ) -> Result<(), CatalogError> {
        if depth > MAX_CATALOG_DEPTH {
            return Err(CatalogError::TooDeep);
        }
        let remaining = MAX_CATALOG_ENTRIES.saturating_sub(self.entries.len());
        let mut children = Vec::with_capacity(remaining.min(1024));
        for child in fs::read_dir(directory)? {
            if children.len() >= remaining {
                return Err(CatalogError::TooManyEntries);
            }
            children.push(child?);
        }
        children.sort_by_key(fs::DirEntry::file_name);
        for child in children {
            let source_path = child.path();
            let metadata = fs::symlink_metadata(&source_path)?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if !metadata.is_file() && !metadata.is_dir() {
                continue;
            }
            let raw_name = child
                .file_name()
                .to_str()
                .map(str::to_owned)
                .ok_or(CatalogError::NonUtf8Path)?;
            if matches!(
                raw_name.as_str(),
                ".quick-share-staging" | ".quick-share-web-upload"
            ) {
                continue;
            }
            let name = safe_display(&raw_name);
            let display_path = format!("{parent_display}/{name}");
            let canonical_path = fs::canonicalize(&source_path)?;
            if !canonical_path.starts_with(canonical_root) {
                return Err(CatalogError::EscapesRoot);
            }
            let id = self.push_entry(
                Some(parent_id.clone()),
                name,
                display_path.clone(),
                if metadata.is_dir() {
                    CatalogEntryKind::Directory
                } else {
                    CatalogEntryKind::File
                },
                if metadata.is_file() {
                    metadata.len()
                } else {
                    0
                },
                source_path.clone(),
                canonical_path,
                canonical_root.to_path_buf(),
            )?;
            if metadata.is_dir() {
                self.walk_directory(&source_path, canonical_root, &id, display_path, depth + 1)?;
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn push_entry(
        &mut self,
        parent_id: Option<CatalogId>,
        name: String,
        display_path: String,
        kind: CatalogEntryKind,
        size: u64,
        source_path: PathBuf,
        canonical_path: PathBuf,
        canonical_root: PathBuf,
    ) -> Result<CatalogId, CatalogError> {
        if self.entries.len() >= MAX_CATALOG_ENTRIES {
            return Err(CatalogError::TooManyEntries);
        }
        if display_path.len() > MAX_DISPLAY_PATH_BYTES {
            return Err(CatalogError::DisplayPathTooLong);
        }
        self.display_bytes = self
            .display_bytes
            .checked_add(name.len())
            .and_then(|bytes| bytes.checked_add(display_path.len()))
            .filter(|bytes| *bytes <= MAX_CATALOG_DISPLAY_BYTES)
            .ok_or(CatalogError::CatalogTooLarge)?;
        let id = CatalogId::generate();
        let media_type = (kind == CatalogEntryKind::File).then(|| {
            mime_guess::from_path(&name)
                .first_or_octet_stream()
                .to_string()
        });
        self.entries.push(CatalogEntry {
            id: id.clone(),
            parent_id,
            name,
            display_path,
            kind,
            size,
            media_type,
            source_path,
            canonical_path,
            canonical_root,
            memory: None,
        });
        Ok(id)
    }
}

fn verify_entry(entry: &CatalogEntry) -> Result<(), CatalogError> {
    let metadata = fs::symlink_metadata(&entry.source_path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(CatalogError::SourceChanged);
    }
    let canonical = fs::canonicalize(&entry.source_path)?;
    if canonical != entry.canonical_path || !canonical.starts_with(&entry.canonical_root) {
        return Err(CatalogError::SourceChanged);
    }
    Ok(())
}

fn safe_display(source: &str) -> String {
    source
        .chars()
        .map(|character| {
            if character.is_control() {
                '\u{fffd}'
            } else {
                character
            }
        })
        .collect()
}

fn unique_top_name(name: &str, seen: &mut BTreeSet<String>) -> String {
    if seen.insert(name.to_owned()) {
        return name.to_owned();
    }
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(name);
    let extension = path.extension().and_then(|value| value.to_str());
    for index in 2..=MAX_CATALOG_ENTRIES {
        let candidate = match extension {
            Some(extension) => format!("{stem} ({index}).{extension}"),
            None => format!("{name} ({index})"),
        };
        if seen.insert(candidate.clone()) {
            return candidate;
        }
    }
    let candidate = format!("{name} ({})", Uuid::now_v7());
    seen.insert(candidate.clone());
    candidate
}

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("at least one share path is required")]
    NoSources,
    #[error("share root cannot be a symbolic link")]
    SymlinkRoot,
    #[error("share source is not a regular file or directory")]
    UnsupportedSource,
    #[error("share path is not valid UTF-8")]
    NonUtf8Path,
    #[error("share catalog exceeds the {MAX_CATALOG_ENTRIES}-entry bound")]
    TooManyEntries,
    #[error("share catalog exceeds the {MAX_CATALOG_DEPTH}-level depth bound")]
    TooDeep,
    #[error("share catalog display metadata exceeds the {MAX_CATALOG_DISPLAY_BYTES}-byte bound")]
    CatalogTooLarge,
    #[error("share catalog display path is too long")]
    DisplayPathTooLong,
    #[error("share source escapes its selected root")]
    EscapesRoot,
    #[error("in-memory catalog filename is invalid")]
    InvalidMemoryName,
    #[error("catalog entry ID is invalid")]
    InvalidId,
    #[error("catalog entry was not found")]
    NotFound,
    #[error("catalog entry is not a downloadable file")]
    NotAFile,
    #[error("catalog source changed after the catalog was built")]
    SourceChanged,
    #[error("share catalog filesystem operation failed")]
    Io(#[from] io::Error),
}
