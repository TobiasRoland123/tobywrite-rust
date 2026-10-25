use crate::text_file::TextFile;
use anyhow::{Context, Result, anyhow, bail};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tempfile::{Builder, NamedTempFile};
use walkdir::WalkDir;

const MAX_DOCUMENT_BYTES: u64 = 32 * 1024 * 1024;
const MAX_INDEXED_FILES: usize = 10_000;
const MAX_SEARCH_RESULTS: usize = 100;
const MAX_RECENT_FILES: usize = 30;
const MAX_STATE_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub appearance: Appearance,
    pub font_size: i32,
    pub focus_mode: bool,
    pub show_syntax: bool,
    pub linked_folders: Vec<PathBuf>,
    pub recent_files: Vec<PathBuf>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            appearance: Appearance::System,
            font_size: 20,
            focus_mode: false,
            show_syntax: false,
            linked_folders: Vec::new(),
            recent_files: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Store {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
}

impl Store {
    pub fn new() -> Result<Self> {
        let dirs = ProjectDirs::from("", "", "tobywrite")
            .ok_or_else(|| anyhow!("could not determine XDG configuration and data directories"))?;
        Ok(Self {
            config_dir: dirs.config_dir().to_path_buf(),
            data_dir: dirs.data_dir().to_path_buf(),
        })
    }

    /// Construct isolated XDG-like directories beneath `root`, intended for tests.
    pub fn at(root: &Path) -> Result<Self> {
        let store = Self {
            config_dir: root.join("config"),
            data_dir: root.join("data"),
        };
        ensure_private_dir(&store.config_dir)?;
        ensure_private_dir(&store.data_dir)?;
        Ok(store)
    }

    pub fn load_preferences(&self) -> Result<Preferences> {
        let path = self.config_dir.join("preferences.json");
        match read_snapshot(&path, MAX_STATE_BYTES)? {
            Some((bytes, _)) => {
                let mut preferences: Preferences =
                    serde_json::from_slice(&bytes).with_context(|| {
                        format!("failed to parse preferences at {}", path.display())
                    })?;
                preferences.font_size = preferences.font_size.clamp(12, 36);
                Ok(preferences)
            }
            None => Ok(Preferences::default()),
        }
    }

    pub fn save_preferences(&self, preferences: &Preferences) -> Result<()> {
        ensure_private_dir(&self.config_dir)?;
        let bytes =
            serde_json::to_vec_pretty(preferences).context("failed to encode preferences")?;
        atomic_write(&self.config_dir.join("preferences.json"), &bytes, None)
    }

    pub fn save_session(&self, drafts: &[Draft]) -> Result<()> {
        ensure_private_dir(&self.data_dir)?;
        let bytes =
            serde_json::to_vec(drafts).context("failed to encode document recovery session")?;
        atomic_write(&self.data_dir.join("session.json"), &bytes, None)
    }

    pub fn load_session(&self) -> Result<Vec<Draft>> {
        let path = self.data_dir.join("session.json");
        match read_snapshot(&path, MAX_STATE_BYTES)? {
            Some((bytes, _)) => serde_json::from_slice(&bytes)
                .with_context(|| format!("failed to parse recovery session at {}", path.display())),
            None => Ok(Vec::new()),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Draft {
    pub path: Option<PathBuf>,
    pub file: TextFile,
    pub dirty: bool,
    pub baseline: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Document {
    pub path: Option<PathBuf>,
    pub file: TextFile,
    pub dirty: bool,
    pub baseline: Option<String>,
}

impl Document {
    pub fn new() -> Self {
        Self {
            path: None,
            file: TextFile::default(),
            dirty: false,
            baseline: None,
        }
    }

    pub fn open(path: &Path) -> Result<Self> {
        let path = canonical_target(path)?;
        let (bytes, snapshot) = read_snapshot(&path, MAX_DOCUMENT_BYTES)?
            .ok_or_else(|| anyhow!("{} no longer exists", path.display()))?;
        let text_file = TextFile::decode(&bytes)
            .with_context(|| format!("failed to decode {}", path.display()))?;
        Ok(Self {
            path: Some(path),
            file: text_file,
            dirty: false,
            baseline: Some(snapshot.digest),
        })
    }

    pub fn from_draft(draft: Draft) -> Self {
        Self {
            path: draft.path,
            file: draft.file,
            dirty: draft.dirty,
            baseline: draft.baseline,
        }
    }

    pub fn draft(&self) -> Draft {
        Draft {
            path: self.path.clone(),
            file: self.file.clone(),
            dirty: self.dirty,
            baseline: self.baseline.clone(),
        }
    }

    pub fn title(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".to_owned())
    }

    pub fn set_text(&mut self, text: String) {
        if self.file.text != text {
            self.file.text = text;
            self.dirty = true;
        }
    }

    /// Whether the on-disk bytes differ from the last successful open or save.
    pub fn disk_changed(&self) -> Result<bool> {
        let Some(path) = &self.path else {
            return Ok(false);
        };
        let snapshot = read_snapshot(path, MAX_DOCUMENT_BYTES)?;
        Ok(self.baseline.as_deref()
            != snapshot
                .as_ref()
                .map(|(_, snapshot)| snapshot.digest.as_str()))
    }

    /// Save atomically. Conflicts and existing Save As destinations require
    /// explicit `overwrite` confirmation.
    pub fn save(&mut self, target: &Path, overwrite: bool) -> Result<()> {
        let target = canonical_target(target)?;
        let same_document = self
            .path
            .as_ref()
            .is_some_and(|path| equivalent_path(path, &target));
        // Hash exactly the bytes observed for this destination. Checking the
        // document's old path separately can race with an external replacement.
        let existing = read_snapshot(&target, MAX_DOCUMENT_BYTES)?.map(|(_, snapshot)| snapshot);
        if same_document
            && !overwrite
            && self.baseline.as_deref()
                != existing.as_ref().map(|snapshot| snapshot.digest.as_str())
        {
            bail!("{} changed on disk since it was opened", target.display());
        }
        if let Some(snapshot) = &existing {
            if !same_document && !overwrite {
                bail!(
                    "{} already exists; confirm replacement first",
                    target.display()
                );
            }
            ensure_replaceable(&target, &snapshot.metadata)?;
        }
        let bytes = self.file.encode();
        if bytes.len() as u64 > MAX_DOCUMENT_BYTES {
            bail!("document exceeds the 32 MiB limit");
        }
        let permissions = existing
            .as_ref()
            .map(|snapshot| snapshot.metadata.permissions());
        let staged = stage_write(&target, &bytes, permissions.as_ref(), existing.as_ref())?;
        commit_staged(staged, &target, existing.as_ref(), MAX_DOCUMENT_BYTES)?;
        self.path = Some(target);
        self.baseline = Some(digest(&bytes));
        self.dirty = false;
        Ok(())
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

/// Commit rendered export bytes with the same protections as a document save.
pub fn save_export_bytes(path: &Path, bytes: &[u8], overwrite: bool) -> Result<()> {
    if bytes.len() as u64 > MAX_DOCUMENT_BYTES {
        bail!("export exceeds the 32 MiB limit");
    }
    let target = canonical_target(path)?;
    let existing = read_snapshot(&target, MAX_DOCUMENT_BYTES)?.map(|(_, snapshot)| snapshot);
    if let Some(snapshot) = &existing {
        if !overwrite {
            bail!(
                "{} already exists; confirm replacement first",
                target.display()
            );
        }
        ensure_replaceable(&target, &snapshot.metadata)?;
    }
    let permissions = existing
        .as_ref()
        .map(|snapshot| snapshot.metadata.permissions());
    let staged = stage_write(&target, bytes, permissions.as_ref(), existing.as_ref())?;
    commit_staged(staged, &target, existing.as_ref(), MAX_DOCUMENT_BYTES)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileEntry {
    pub path: PathBuf,
    pub title: String,
    pub modified: u64,
}

/// Discover plain text and Markdown files, without following symlinks.
pub fn discover_files(folders: &[PathBuf], recents: &[PathBuf]) -> Vec<FileEntry> {
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    let mut recent_rank = HashMap::new();
    for (rank, path) in recents.iter().take(MAX_INDEXED_FILES).enumerate() {
        if !supported_extension(path) {
            continue;
        }
        // A recent file can be outside every linked folder. Use the same
        // canonical identity below to avoid showing its folder entry twice.
        if let Ok(path) = canonical_target(path) {
            recent_rank.entry(path.clone()).or_insert(rank);
            if fs::metadata(&path).is_ok_and(|metadata| metadata.is_file())
                && seen.insert(path.clone())
            {
                files.push(file_entry(path));
            }
        }
    }
    for folder in folders {
        if files.len() >= MAX_INDEXED_FILES {
            break;
        }
        for entry in WalkDir::new(folder)
            .follow_links(false)
            .max_depth(10)
            .into_iter()
            .filter_entry(|entry| {
                !(entry.depth() > 0
                    && entry.file_type().is_dir()
                    && entry.file_name().to_string_lossy().starts_with('.'))
            })
            .filter_map(Result::ok)
        {
            if entry.file_type().is_symlink() {
                continue;
            }
            if entry.file_type().is_dir() {
                if entry.depth() > 0 && entry.file_name().to_string_lossy().starts_with('.') {
                    continue;
                }
                continue;
            }
            if !entry.file_type().is_file() || !supported_extension(entry.path()) {
                continue;
            }
            let Ok(path) = canonical_target(entry.path()) else {
                continue;
            };
            if !seen.insert(path.clone()) {
                continue;
            }
            files.push(file_entry(path));
            if files.len() >= MAX_INDEXED_FILES {
                break;
            }
        }
        if files.len() >= MAX_INDEXED_FILES {
            break;
        }
    }
    files.sort_by(|a, b| {
        let ar = recent_rank.get(&a.path);
        let br = recent_rank.get(&b.path);
        match (ar, br) {
            (Some(x), Some(y)) => x.cmp(y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            _ => b.modified.cmp(&a.modified),
        }
    });
    files
}

/// Rank matching files using filename/title/path tokens and cap results at 100.
pub fn search_files(files: &[FileEntry], query: &str) -> Vec<FileEntry> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return files.iter().take(MAX_SEARCH_RESULTS).cloned().collect();
    }
    let tokens: Vec<&str> = query.split_whitespace().collect();
    let mut scored: Vec<(i32, &FileEntry)> = files
        .iter()
        .filter_map(|file| {
            let name = file
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();
            let title = file.title.to_lowercase();
            let path = file.path.to_string_lossy().to_lowercase();
            let score = if title == query || name == query {
                1000
            } else if title.starts_with(&query) {
                600
            } else if name.starts_with(&query) {
                500
            } else if title.contains(&query) {
                300
            } else if name.contains(&query) {
                200
            } else if path.contains(&query) {
                100
            } else if tokens
                .iter()
                .all(|t| title.contains(t) || name.contains(t) || path.contains(t))
            {
                80
            } else if is_subsequence(&query, &title) {
                40
            } else {
                0
            };
            (score > 0).then_some((score + recency_bonus(file.modified), file))
        })
        .collect();
    scored.sort_by(|(ascore, a), (bscore, b)| {
        bscore.cmp(ascore).then_with(|| b.modified.cmp(&a.modified))
    });
    scored
        .into_iter()
        .take(MAX_SEARCH_RESULTS)
        .map(|(_, file)| file.clone())
        .collect()
}

pub fn remember_recent(preferences: &mut Preferences, path: &Path) {
    let path = path.to_path_buf();
    preferences.recent_files.retain(|p| p != &path);
    preferences.recent_files.insert(0, path);
    preferences.recent_files.truncate(MAX_RECENT_FILES);
}

fn file_entry(path: PathBuf) -> FileEntry {
    let title = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let modified = fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    FileEntry {
        path,
        title,
        modified,
    }
}

fn supported_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "md" | "markdown" | "mdown" | "txt" | "text"
            )
        })
        .unwrap_or(false)
}

fn recency_bonus(modified: u64) -> i32 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let days = now.saturating_sub(modified) / 86_400;
    30i32.saturating_sub(days.min(30) as i32)
}

fn is_subsequence(query: &str, target: &str) -> bool {
    let mut chars = target.chars();
    query.chars().all(|q| chars.by_ref().any(|c| c == q))
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn canonical_target(path: &Path) -> Result<PathBuf> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            return fs::canonicalize(path)
                .with_context(|| format!("failed to resolve {}", path.display()));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("failed to inspect {}", path.display()));
        }
    }
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent directory", path.display()))?;
    let parent = if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    };
    let parent = fs::canonicalize(parent)
        .with_context(|| format!("parent folder {} does not exist", parent.display()))?;
    let name = path
        .file_name()
        .ok_or_else(|| anyhow!("{} has no filename", path.display()))?;
    Ok(parent.join(name))
}

fn equivalent_path(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn ensure_private_dir(path: &Path) -> Result<()> {
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        bail!(
            "{} is a symlink; refusing to change another directory's permissions",
            path.display()
        );
    }
    fs::create_dir_all(path).with_context(|| format!("failed to create {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .with_context(|| format!("failed to secure {}", path.display()))?;
    }
    Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8], permissions: Option<&fs::Permissions>) -> Result<()> {
    if bytes.len() as u64 > MAX_STATE_BYTES {
        bail!("state exceeds the 128 MiB recovery limit");
    }
    let existing = read_snapshot(path, MAX_STATE_BYTES)?.map(|(_, snapshot)| snapshot);
    if let Some(snapshot) = &existing {
        ensure_replaceable(path, &snapshot.metadata)?;
    }
    let staged = stage_write(path, bytes, permissions, existing.as_ref())?;
    commit_staged(staged, path, existing.as_ref(), MAX_STATE_BYTES)
}

#[derive(Debug)]
struct DiskSnapshot {
    digest: String,
    metadata: fs::Metadata,
}

impl DiskSnapshot {
    fn matches(&self, other: &Self) -> bool {
        self.digest == other.digest && same_metadata(&self.metadata, &other.metadata)
    }
}

// Read bounded bytes and their metadata from one open descriptor. A separate
// stat followed by read can associate a fingerprint with a different inode.
fn read_snapshot(path: &Path, limit: u64) -> Result<Option<(Vec<u8>, DiskSnapshot)>> {
    let named_metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to inspect {}", path.display()));
        }
    };
    if !named_metadata.is_file() {
        bail!(
            "{} is not a regular file, or changed to a symlink",
            path.display()
        );
    }
    if named_metadata.len() > limit {
        bail!(
            "{} exceeds the {} MiB size limit",
            path.display(),
            limit / 1024 / 1024
        );
    }
    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let before = file.metadata()?;
    if !same_metadata(&named_metadata, &before) {
        bail!("{} changed while it was being opened", path.display());
    }
    let mut bytes = Vec::with_capacity(before.len() as usize);
    (&file)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read {}", path.display()))?;
    if bytes.len() as u64 > limit {
        bail!(
            "{} exceeds the {} MiB size limit",
            path.display(),
            limit / 1024 / 1024
        );
    }
    let after = file.metadata()?;
    let current = fs::symlink_metadata(path)
        .with_context(|| format!("{} disappeared while it was being read", path.display()))?;
    if !same_metadata(&before, &after) || !same_metadata(&after, &current) {
        bail!(
            "{} changed while it was being read; try again",
            path.display()
        );
    }
    let fingerprint = digest(&bytes);
    Ok(Some((
        bytes,
        DiskSnapshot {
            digest: fingerprint,
            metadata: after,
        },
    )))
}

fn same_metadata(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    if !a.is_file()
        || !b.is_file()
        || a.len() != b.len()
        || a.modified().ok() != b.modified().ok()
        || a.permissions().readonly() != b.permissions().readonly()
    {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.dev() == b.dev()
            && a.ino() == b.ino()
            && a.nlink() == b.nlink()
            && a.mode() == b.mode()
            && a.uid() == b.uid()
            && a.gid() == b.gid()
            && a.ctime() == b.ctime()
            && a.ctime_nsec() == b.ctime_nsec()
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn ensure_replaceable(path: &Path, metadata: &fs::Metadata) -> Result<()> {
    if metadata.permissions().readonly() {
        bail!("{} is read-only; choose Save As instead", path.display());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() > 1 {
            bail!(
                "{} has multiple hard links; choose Save As to avoid breaking its aliases",
                path.display()
            );
        }
    }
    // Rename only needs directory write permission. Explicitly opening the
    // original for writing also respects its ACL and file permissions.
    OpenOptions::new()
        .write(true)
        .open(path)
        .with_context(|| format!("{} is not writable; choose Save As instead", path.display()))?;
    Ok(())
}

fn stage_write(
    path: &Path,
    bytes: &[u8],
    permissions: Option<&fs::Permissions>,
    existing: Option<&DiskSnapshot>,
) -> Result<NamedTempFile> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent directory", path.display()))?;
    let mut temp = Builder::new()
        .prefix(".tobywrite-")
        .tempfile_in(parent)
        .with_context(|| format!("failed to create temporary file in {}", parent.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt, chown};
        if let Some(existing) = existing {
            let staged_metadata = temp.as_file().metadata()?;
            if staged_metadata.uid() != existing.metadata.uid()
                || staged_metadata.gid() != existing.metadata.gid()
            {
                chown(
                    temp.path(),
                    Some(existing.metadata.uid()),
                    Some(existing.metadata.gid()),
                )
                .with_context(|| {
                    format!(
                        "cannot preserve the ownership of {}; choose Save As instead",
                        path.display()
                    )
                })?;
            }
        }
        let mode = permissions
            .map(|p| {
                use std::os::unix::fs::PermissionsExt;
                p.mode() & 0o777
            })
            .unwrap_or(0o600);
        temp.as_file()
            .set_permissions(fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    if let Some(permissions) = permissions {
        temp.as_file().set_permissions(permissions.clone())?;
    }
    temp.write_all(bytes)
        .with_context(|| format!("failed to write {}", path.display()))?;
    temp.as_file()
        .sync_all()
        .with_context(|| format!("failed to sync {}", path.display()))?;
    Ok(temp)
}

fn commit_staged(
    temp: NamedTempFile,
    path: &Path,
    expected: Option<&DiskSnapshot>,
    limit: u64,
) -> Result<()> {
    // Recheck after writing and syncing the temporary file, immediately before
    // committing it. Even an explicit overwrite authorizes only the version
    // observed at the start of this save operation.
    let current = read_snapshot(path, limit)?.map(|(_, snapshot)| snapshot);
    let unchanged = match (expected, current.as_ref()) {
        (None, None) => true,
        (Some(expected), Some(current)) => expected.matches(current),
        _ => false,
    };
    if !unchanged {
        bail!(
            "{} changed during the save; its current contents were kept",
            path.display()
        );
    }
    if expected.is_some() {
        temp.persist(path)
            .map_err(|error| error.error)
            .with_context(|| format!("failed to atomically replace {}", path.display()))?;
    } else {
        // Creation must never overwrite a file that appeared after our check.
        temp.persist_noclobber(path)
            .map_err(|error| error.error)
            .with_context(|| {
                format!(
                    "failed to create {} without replacing an existing file",
                    path.display()
                )
            })?;
    }
    let parent = path.parent().expect("staged paths have a parent");
    if let Ok(dir) = File::open(parent) {
        let _ = dir.sync_all();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn document_state(document: &Document) -> serde_json::Value {
        serde_json::to_value(document.draft()).unwrap()
    }

    #[test]
    fn preferences_bound_the_font_size_before_layout_arithmetic() {
        let dir = tempdir().unwrap();
        let store = Store::at(dir.path()).unwrap();
        let preferences = Preferences {
            font_size: i32::MAX,
            ..Default::default()
        };
        store.save_preferences(&preferences).unwrap();
        assert_eq!(store.load_preferences().unwrap().font_size, 36);
    }

    #[cfg(unix)]
    #[test]
    fn exports_preserve_existing_bytes_without_confirmation_or_write_permission() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let path = dir.path().join("document.pdf");
        fs::write(&path, b"original PDF bytes").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        assert!(save_export_bytes(&path, b"replacement PDF", false).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"original PDF bytes");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o440)).unwrap();
        assert!(save_export_bytes(&path, b"replacement PDF", true).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"original PDF bytes");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o440
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        save_export_bytes(&path, b"replacement PDF", true).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"replacement PDF");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        let fresh = dir.path().join("fresh.pdf");
        save_export_bytes(&fresh, b"new PDF", false).unwrap();
        assert_eq!(fs::read(fresh).unwrap(), b"new PDF");
    }

    #[test]
    fn deleted_original_requires_confirmation_and_failed_save_keeps_the_draft() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("deleted.md");
        fs::write(&path, b"original").unwrap();
        let mut document = Document::open(&path).unwrap();
        document.set_text("unsaved work".into());
        let before = document_state(&document);
        fs::remove_file(&path).unwrap();
        assert!(document.disk_changed().unwrap());
        assert!(document.save(&path, false).is_err());
        assert!(!path.exists());
        assert_eq!(document_state(&document), before);
        document.save(&path, true).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"unsaved work");
        assert!(!document.disk_changed().unwrap());
    }

    #[test]
    fn save_as_protects_existing_destination_and_keeps_the_source() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source.md");
        let destination = dir.path().join("destination.md");
        fs::write(&source, b"source original").unwrap();
        fs::write(&destination, b"destination original").unwrap();
        let mut document = Document::open(&source).unwrap();
        document.set_text("edited draft".into());
        let before = document_state(&document);
        assert!(document.save(&destination, false).is_err());
        assert_eq!(document_state(&document), before);
        assert_eq!(fs::read(&source).unwrap(), b"source original");
        assert_eq!(fs::read(&destination).unwrap(), b"destination original");
        document.save(&destination, true).unwrap();
        assert_eq!(fs::read(&source).unwrap(), b"source original");
        assert_eq!(fs::read(&destination).unwrap(), b"edited draft");
        assert_eq!(document.path.as_deref(), Some(destination.as_path()));
    }

    #[test]
    fn fingerprints_track_original_encoded_bytes_and_recovered_conflicts() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("encoded.md");
        let original = b"\xef\xbb\xbfone\r\ntwo\r\n";
        fs::write(&path, original).unwrap();
        let mut document = Document::open(&path).unwrap();
        assert_eq!(
            document.baseline.as_deref(),
            Some(digest(original).as_str())
        );
        assert!(!document.disk_changed().unwrap());
        document.set_text("edited\n".into());
        let mut restored = Document::from_draft(document.draft());
        fs::write(&path, b"external edit").unwrap();
        let before = document_state(&restored);
        assert!(restored.save(&path, false).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"external edit");
        assert_eq!(document_state(&restored), before);
        restored.save(&path, true).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"\xef\xbb\xbfedited\r\n");
        assert_eq!(
            restored.baseline.as_deref(),
            Some(digest(&fs::read(&path).unwrap()).as_str())
        );
        assert!(!restored.disk_changed().unwrap());
    }

    #[test]
    fn recheck_refuses_edits_deletions_and_creations_during_staging() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("racing.md");
        fs::write(&path, b"original").unwrap();
        let (_, expected) = read_snapshot(&path, MAX_DOCUMENT_BYTES).unwrap().unwrap();
        let staged = stage_write(&path, b"my edit", None, Some(&expected)).unwrap();
        fs::write(&path, b"external latest edit").unwrap();
        assert!(commit_staged(staged, &path, Some(&expected), MAX_DOCUMENT_BYTES).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"external latest edit");

        let (_, expected) = read_snapshot(&path, MAX_DOCUMENT_BYTES).unwrap().unwrap();
        let staged = stage_write(&path, b"my edit", None, Some(&expected)).unwrap();
        fs::remove_file(&path).unwrap();
        assert!(commit_staged(staged, &path, Some(&expected), MAX_DOCUMENT_BYTES).is_err());
        assert!(!path.exists());

        let staged = stage_write(&path, b"my edit", None, None).unwrap();
        fs::write(&path, b"new external file").unwrap();
        assert!(commit_staged(staged, &path, None, MAX_DOCUMENT_BYTES).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"new external file");
    }

    #[cfg(unix)]
    #[test]
    fn readonly_files_and_hardlinks_cannot_be_silently_replaced() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let path = dir.path().join("original.md");
        fs::write(&path, b"original bytes").unwrap();
        let mut document = Document::open(&path).unwrap();
        document.set_text("unsaved edit".into());
        let before = document_state(&document);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        assert!(document.save(&path, true).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"original bytes");
        assert_eq!(document_state(&document), before);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o444
        );

        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        let alias = dir.path().join("hardlink.md");
        fs::hard_link(&path, &alias).unwrap();
        for target in [&path, &alias] {
            assert!(document.save(target, true).is_err());
            assert_eq!(fs::read(&path).unwrap(), b"original bytes");
            assert_eq!(fs::read(&alias).unwrap(), b"original bytes");
            assert_eq!(document_state(&document), before);
        }
        let copy = dir.path().join("copy.md");
        document.save(&copy, false).unwrap();
        assert_eq!(fs::read(&copy).unwrap(), b"unsaved edit");
        assert_eq!(fs::read(&path).unwrap(), b"original bytes");
        assert_eq!(fs::read(&alias).unwrap(), b"original bytes");
    }

    #[cfg(unix)]
    #[test]
    fn retargeted_symlinks_and_same_content_inode_replacements_are_detected() {
        use std::os::unix::fs::symlink;
        let dir = tempdir().unwrap();
        let path = dir.path().join("target.md");
        let other = dir.path().join("other.md");
        fs::write(&path, b"original").unwrap();
        fs::write(&other, b"other file").unwrap();
        let (_, expected) = read_snapshot(&path, MAX_DOCUMENT_BYTES).unwrap().unwrap();
        let staged = stage_write(&path, b"my edit", None, Some(&expected)).unwrap();
        fs::remove_file(&path).unwrap();
        symlink(&other, &path).unwrap();
        assert!(commit_staged(staged, &path, Some(&expected), MAX_DOCUMENT_BYTES).is_err());
        assert_eq!(fs::read(&other).unwrap(), b"other file");
        assert!(
            fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
        );

        fs::remove_file(&path).unwrap();
        fs::write(&path, b"original").unwrap();
        let (_, expected) = read_snapshot(&path, MAX_DOCUMENT_BYTES).unwrap().unwrap();
        let staged = stage_write(&path, b"my edit", None, Some(&expected)).unwrap();
        let replacement = dir.path().join("replacement.md");
        fs::write(&replacement, b"original").unwrap();
        fs::rename(replacement, &path).unwrap();
        assert!(commit_staged(staged, &path, Some(&expected), MAX_DOCUMENT_BYTES).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"original");
    }

    #[cfg(unix)]
    #[test]
    fn recovery_errors_leave_original_bytes_and_symlink_targets_intact() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = tempdir().unwrap();
        let store = Store::at(dir.path()).unwrap();
        let session = store.data_dir.join("session.json");
        fs::write(&session, b"[]").unwrap();
        fs::set_permissions(&session, fs::Permissions::from_mode(0o400)).unwrap();
        assert!(store.save_session(&[]).is_err());
        assert_eq!(fs::read(&session).unwrap(), b"[]");
        fs::remove_file(&session).unwrap();
        let other = dir.path().join("valuable.json");
        fs::write(&other, b"valuable original bytes").unwrap();
        symlink(&other, &session).unwrap();
        assert!(store.load_session().is_err());
        assert!(store.save_session(&[]).is_err());
        assert_eq!(fs::read(&other).unwrap(), b"valuable original bytes");
        assert!(
            fs::symlink_metadata(&session)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[cfg(unix)]
    #[test]
    fn recovery_directory_symlinks_do_not_change_other_directory_permissions() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = tempdir().unwrap();
        let external = dir.path().join("external-folder");
        fs::create_dir(&external).unwrap();
        fs::set_permissions(&external, fs::Permissions::from_mode(0o755)).unwrap();
        let alias = dir.path().join("data-alias");
        symlink(&external, &alias).unwrap();
        assert!(ensure_private_dir(&alias).is_err());
        assert_eq!(
            fs::metadata(&external).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }

    #[test]
    fn oversized_files_fail_without_replacing_their_original_bytes() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("large.md");
        fs::write(&path, b"original prefix").unwrap();
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(MAX_DOCUMENT_BYTES + 1)
            .unwrap();
        let mut document = Document::new();
        document.set_text("unsaved edit".into());
        let before = document_state(&document);
        assert!(Document::open(&path).is_err());
        assert!(document.save(&path, true).is_err());
        assert_eq!(document_state(&document), before);
        assert_eq!(fs::metadata(&path).unwrap().len(), MAX_DOCUMENT_BYTES + 1);
        let mut prefix = [0; 15];
        File::open(&path).unwrap().read_exact(&mut prefix).unwrap();
        assert_eq!(&prefix, b"original prefix");
    }

    #[test]
    fn relative_new_filename_resolves_against_the_working_directory() {
        let name = format!("tobywrite-new-{}-relative.md", std::process::id());
        assert_eq!(
            canonical_target(Path::new(&name)).unwrap(),
            std::env::current_dir().unwrap().join(name)
        );
    }

    #[test]
    fn discovery_includes_recent_files_outside_linked_folders_without_duplicates() {
        let dir = tempdir().unwrap();
        let linked = dir.path().join("linked");
        fs::create_dir(&linked).unwrap();
        let outside = dir.path().join("outside.md");
        let inside = linked.join("inside.md");
        let missing = dir.path().join("missing.md");
        fs::write(&outside, b"outside").unwrap();
        fs::write(&inside, b"inside").unwrap();
        let recents = vec![outside.clone(), inside.clone(), missing, outside.clone()];
        let files = discover_files(&[], &recents);
        assert_eq!(
            files.iter().map(|file| &file.path).collect::<Vec<_>>(),
            vec![&outside, &inside]
        );
        let files = discover_files(&[linked], &recents);
        assert_eq!(
            files.iter().map(|file| &file.path).collect::<Vec<_>>(),
            vec![&outside, &inside]
        );
    }

    #[test]
    fn save_detects_external_edits_and_confirmation_replaces_atomically() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("note.md");
        fs::write(&path, b"original\n").unwrap();
        let mut document = Document::open(&path).unwrap();
        document.set_text("my edit\n".into());
        fs::write(&path, b"external change\n").unwrap();
        assert!(document.disk_changed().unwrap());
        assert!(document.save(&path, false).is_err());
        assert_eq!(document.file.text, "my edit\n");
        assert!(document.dirty);
        assert_eq!(fs::read(&path).unwrap(), b"external change\n");

        document.save(&path, true).unwrap();
        assert!(!document.dirty);
        assert!(!document.disk_changed().unwrap());
        assert_eq!(fs::read(&path).unwrap(), b"my edit\n");
    }

    #[test]
    fn failed_save_and_corrupt_recovery_preserve_state_and_bytes() {
        let dir = tempdir().unwrap();
        let mut document = Document::new();
        document.set_text("unsaved".into());
        let missing = dir.path().join("missing").join("note.md");
        assert!(document.save(&missing, true).is_err());
        assert!(document.path.is_none());
        assert!(document.dirty);
        assert_eq!(document.file.text, "unsaved");

        let store = Store::at(dir.path()).unwrap();
        let session_path = store.data_dir.join("session.json");
        fs::write(&session_path, b"{ broken json").unwrap();
        assert!(store.load_session().is_err());
        assert_eq!(fs::read(session_path).unwrap(), b"{ broken json");
    }

    #[test]
    fn session_round_trips_drafts_and_private_state_files() {
        let dir = tempdir().unwrap();
        let store = Store::at(dir.path()).unwrap();
        let draft = Draft {
            path: None,
            file: TextFile::decode(b"hello\r\n").unwrap(),
            dirty: true,
            baseline: None,
        };
        store.save_session(std::slice::from_ref(&draft)).unwrap();
        assert_eq!(store.load_session().unwrap()[0].file, draft.file);
        store.save_preferences(&Preferences::default()).unwrap();
        assert_eq!(store.load_preferences().unwrap().font_size, 20);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(store.data_dir.join("session.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(store.config_dir.join("preferences.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(&store.data_dir).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn saves_through_symlinks_and_preserves_target_permissions() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = tempdir().unwrap();
        let real = dir.path().join("real.md");
        let link = dir.path().join("alias.md");
        fs::write(&real, b"before").unwrap();
        fs::set_permissions(&real, fs::Permissions::from_mode(0o640)).unwrap();
        symlink(&real, &link).unwrap();
        let mut document = Document::open(&link).unwrap();
        assert_eq!(document.path.as_deref(), Some(real.as_path()));
        document.set_text("after".into());
        document.save(&link, true).unwrap();
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(&real).unwrap(), b"after");
        assert_eq!(
            fs::metadata(&real).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }

    #[test]
    fn discovery_excludes_hidden_folders_and_search_ranks_capped_results() {
        let dir = tempdir().unwrap();
        let notes = dir.path().join("notes");
        let hidden = notes.join(".private");
        fs::create_dir_all(&hidden).unwrap();
        fs::write(notes.join("MeetingNotes.md"), b"notes").unwrap();
        fs::write(notes.join("other.txt"), b"notes").unwrap();
        fs::write(hidden.join("MeetingNotesSecret.md"), b"notes").unwrap();
        let files = discover_files(&[notes], &[]);
        assert_eq!(files.len(), 2);
        assert_eq!(
            search_files(&files, "MeetingNotes")[0].title,
            "MeetingNotes"
        );
        assert!(search_files(&files, "not-present").is_empty());
        let many: Vec<_> = (0..150)
            .map(|n| FileEntry {
                path: PathBuf::from(format!("/notes/note-{n}.md")),
                title: format!("note-{n}"),
                modified: n,
            })
            .collect();
        assert_eq!(search_files(&many, "note").len(), MAX_SEARCH_RESULTS);
    }

    #[test]
    fn recent_files_are_unique_and_limited() {
        let mut prefs = Preferences::default();
        for n in 0..35 {
            remember_recent(&mut prefs, Path::new(&format!("/note-{n}.md")));
        }
        remember_recent(&mut prefs, Path::new("/note-10.md"));
        assert_eq!(prefs.recent_files.len(), MAX_RECENT_FILES);
        assert_eq!(prefs.recent_files[0], PathBuf::from("/note-10.md"));
    }
}
