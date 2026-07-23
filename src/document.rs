use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::SystemTime,
};

static SAVE_COUNTER: AtomicU64 = AtomicU64::new(0);

pub const WELCOME_DOCUMENT: &str = r#"# Welcome to MarkGuin

A focused Markdown editor built in Rust.

## Start writing

- Edit Markdown on the left
- See a clean preview on the right
- Navigate long documents from the outline
- Insert tables and common syntax from the toolbar

> MarkGuin keeps the source visible and puts writing first.

```rust
fn main() {
    println!("Hello, MarkGuin!");
}
```
"#;

#[derive(Debug)]
pub struct Document {
    pub text: String,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    disk_hash: Option<u64>,
    disk_stamp: Option<DiskStamp>,
}

/// Cheap file metadata used to skip reading the whole file when polling for
/// external changes. A metadata miss falls back to a full content hash, so a
/// metadata-only touch is never misreported as a change.
#[derive(Clone, Copy, Debug)]
struct DiskStamp {
    len: u64,
    modified: Option<SystemTime>,
}

impl DiskStamp {
    fn of(path: &Path) -> Option<Self> {
        let metadata = fs::metadata(path).ok()?;
        Some(Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
        })
    }

    fn matches(&self, metadata: &fs::Metadata) -> bool {
        self.len == metadata.len()
            && self.modified.is_some()
            && self.modified == metadata.modified().ok()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiskState {
    Unchanged,
    Changed,
    Missing,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            text: WELCOME_DOCUMENT.into(),
            path: None,
            dirty: false,
            disk_hash: None,
            disk_stamp: None,
        }
    }
}

impl Document {
    pub fn title(&self) -> String {
        self.path
            .as_deref()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .unwrap_or("Untitled.md")
            .to_owned()
    }

    pub fn open(path: PathBuf) -> io::Result<Self> {
        let text = fs::read_to_string(&path)?;
        let disk_stamp = DiskStamp::of(&path);
        Ok(Self {
            disk_hash: Some(content_hash(&text)),
            disk_stamp,
            text,
            path: Some(path),
            dirty: false,
        })
    }

    pub fn recovered(text: String, path: Option<PathBuf>, dirty: bool) -> Self {
        let disk_hash = path
            .as_deref()
            .and_then(|path| fs::read_to_string(path).ok())
            .map(|text| content_hash(&text));
        let disk_stamp = path.as_deref().and_then(DiskStamp::of);
        Self {
            text,
            path,
            dirty,
            disk_hash,
            disk_stamp,
        }
    }

    pub fn save(&mut self) -> io::Result<()> {
        let Some(path) = self.path.as_deref() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "no file path selected",
            ));
        };
        // Write to a temporary file in the same directory and atomically rename
        // it over the destination, so a crash mid-save can never truncate the
        // existing file on disk.
        let mut tmp = path.to_path_buf();
        tmp.set_extension(format!(
            "tmp-{}-{}-{}",
            std::process::id(),
            SAVE_COUNTER.fetch_add(1, Ordering::Relaxed),
            content_hash(&self.text)
        ));
        fs::write(&tmp, &self.text)?;
        if let Ok(file) = fs::OpenOptions::new().write(true).open(&tmp) {
            let _ = file.sync_all();
        }
        fs::rename(&tmp, path)?;
        self.disk_hash = Some(content_hash(&self.text));
        self.disk_stamp = DiskStamp::of(path);
        self.dirty = false;
        Ok(())
    }

    pub fn save_as(&mut self, path: PathBuf) -> io::Result<()> {
        self.path = Some(path);
        self.save()
    }

    pub fn word_count(&self) -> usize {
        self.text.split_whitespace().count()
    }

    pub fn line_count(&self) -> usize {
        if self.text.is_empty() {
            1
        } else {
            self.text.matches('\n').count() + 1
        }
    }

    pub fn disk_state(&self) -> io::Result<DiskState> {
        let Some(path) = self.path.as_deref() else {
            return Ok(DiskState::Unchanged);
        };
        // Cheap pre-check: unchanged metadata means unchanged content, so the
        // full read and hash below only runs when the file might have changed.
        match fs::metadata(path) {
            Ok(metadata) => {
                if self
                    .disk_stamp
                    .is_some_and(|stamp| stamp.matches(&metadata))
                {
                    return Ok(DiskState::Unchanged);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(DiskState::Missing);
            }
            Err(error) => return Err(error),
        }
        match fs::read_to_string(path) {
            Ok(text) => Ok(if Some(content_hash(&text)) == self.disk_hash {
                DiskState::Unchanged
            } else {
                DiskState::Changed
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(DiskState::Missing),
            Err(error) => Err(error),
        }
    }

    pub fn acknowledge_disk(&mut self) -> io::Result<()> {
        self.disk_hash = match self.path.as_deref() {
            Some(path) => {
                self.disk_stamp = DiskStamp::of(path);
                Some(content_hash(&fs::read_to_string(path)?))
            }
            None => None,
        };
        Ok(())
    }
}

fn content_hash(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_words_and_lines() {
        let doc = Document {
            text: "one two\nthree".into(),
            path: None,
            dirty: false,
            disk_hash: None,
            disk_stamp: None,
        };
        assert_eq!(doc.word_count(), 3);
        assert_eq!(doc.line_count(), 2);
    }

    #[test]
    fn detects_changed_and_missing_files_by_content() {
        let path =
            std::env::temp_dir().join(format!("markguin-document-{}.md", std::process::id()));
        fs::write(&path, "original").unwrap();
        let mut doc = Document::open(path.clone()).unwrap();
        assert_eq!(doc.disk_state().unwrap(), DiskState::Unchanged);

        fs::write(&path, "external change").unwrap();
        assert_eq!(doc.disk_state().unwrap(), DiskState::Changed);
        doc.acknowledge_disk().unwrap();
        assert_eq!(doc.disk_state().unwrap(), DiskState::Unchanged);

        fs::remove_file(&path).unwrap();
        assert_eq!(doc.disk_state().unwrap(), DiskState::Missing);
    }

    #[test]
    fn rewriting_identical_content_is_not_reported_as_changed() {
        let path = std::env::temp_dir().join(format!("markguin-rewrite-{}.md", std::process::id()));
        fs::write(&path, "same content").unwrap();
        let doc = Document::open(path.clone()).unwrap();
        assert_eq!(doc.disk_state().unwrap(), DiskState::Unchanged);

        // Rewriting identical content bumps the file metadata; the content
        // hash fallback must still report the file as unchanged.
        std::thread::sleep(std::time::Duration::from_millis(5));
        fs::write(&path, "same content").unwrap();
        assert_eq!(doc.disk_state().unwrap(), DiskState::Unchanged);

        fs::remove_file(&path).ok();
    }
}
