use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    io,
    path::{Path, PathBuf},
};

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
        Ok(Self {
            disk_hash: Some(content_hash(&text)),
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
        Self {
            text,
            path,
            dirty,
            disk_hash,
        }
    }

    pub fn save(&mut self) -> io::Result<()> {
        match self.path.as_deref() {
            Some(path) => {
                fs::write(path, &self.text)?;
                self.disk_hash = Some(content_hash(&self.text));
                self.dirty = false;
                Ok(())
            }
            None => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "no file path selected",
            )),
        }
    }

    pub fn save_as(&mut self, path: PathBuf) -> io::Result<()> {
        self.path = Some(path);
        self.save()
    }

    pub fn word_count(&self) -> usize {
        self.text.split_whitespace().count()
    }

    pub fn line_count(&self) -> usize {
        self.text.lines().count().max(1)
    }

    pub fn disk_state(&self) -> io::Result<DiskState> {
        let Some(path) = self.path.as_deref() else {
            return Ok(DiskState::Unchanged);
        };
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
            Some(path) => Some(content_hash(&fs::read_to_string(path)?)),
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
}
