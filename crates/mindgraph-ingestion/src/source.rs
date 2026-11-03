//! Source-file storage.
//!
//! Uploaded files are stored by [`DataSourceId`], never by the user's
//! filename, which is metadata only. This rules out path traversal and name
//! collisions. [`LocalSourceStore`] lays files out as
//! `<data_dir>/sources/<data_source_id>/source` and writes them atomically:
//! bytes go to a randomly named temporary file in the same directory, which
//! is flushed, synced and renamed on commit, and removed if not committed.

use std::collections::HashMap;
use std::io::{BufReader, Cursor, Read};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use mindgraph_domain::DataSourceId;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::IngestionError;

/// Size and SHA-256 of an upload, computed while it streams.
#[derive(Debug, Default)]
pub struct UploadDigest {
    hasher: Sha256,
    size: u64,
}

impl UploadDigest {
    /// Starts an empty digest.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a chunk.
    pub fn update(&mut self, chunk: &[u8]) {
        self.hasher.update(chunk);
        self.size += chunk.len() as u64;
    }

    /// Bytes seen so far.
    #[must_use]
    pub const fn size(&self) -> u64 {
        self.size
    }

    /// The total size and the lowercase hex SHA-256.
    #[must_use]
    pub fn finish(self) -> (u64, String) {
        (self.size, hex::encode(self.hasher.finalize()))
    }
}

/// Receives the bytes of one new source file.
#[async_trait]
pub trait SourceWriter: Send {
    /// Appends a chunk.
    async fn write(&mut self, chunk: &[u8]) -> Result<(), IngestionError>;
    /// Makes the file durable and visible. Dropping a writer without
    /// committing discards the bytes.
    async fn commit(self: Box<Self>) -> Result<(), IngestionError>;
}

/// Storage of uploaded source files.
#[async_trait]
pub trait SourceStore: Send + Sync {
    /// A stable, backend-relative key for the file (stored as metadata).
    fn storage_key(&self, id: DataSourceId) -> String;
    /// Starts writing the file of a new data source.
    async fn create(&self, id: DataSourceId) -> Result<Box<dyn SourceWriter>, IngestionError>;
    /// Opens a committed file for streaming reads.
    async fn open(&self, id: DataSourceId) -> Result<Box<dyn Read + Send>, IngestionError>;
    /// Deletes a file. Deleting a missing file is not an error.
    async fn delete(&self, id: DataSourceId) -> Result<(), IngestionError>;
}

/// Source files on the local filesystem (v0.1 runtime implementation).
#[derive(Debug, Clone)]
pub struct LocalSourceStore {
    root: PathBuf,
}

impl LocalSourceStore {
    /// Uses `<data_dir>/sources`, creating it if needed.
    ///
    /// # Errors
    ///
    /// [`IngestionError::SourceStore`] if the directory cannot be created.
    pub async fn new(data_dir: impl Into<PathBuf>) -> Result<Self, IngestionError> {
        let root = data_dir.into().join("sources");
        tokio::fs::create_dir_all(&root)
            .await
            .map_err(|err| IngestionError::storage("cannot create the source directory", err))?;
        Ok(Self { root })
    }

    fn dir(&self, id: DataSourceId) -> PathBuf {
        self.root.join(id.to_string())
    }
}

struct LocalWriter {
    file: Option<tokio::fs::File>,
    temp: PathBuf,
    target: PathBuf,
    committed: bool,
}

impl Drop for LocalWriter {
    fn drop(&mut self) {
        if !self.committed {
            // Best effort: an abandoned upload must not leave a partial file.
            let _ = std::fs::remove_file(&self.temp);
        }
    }
}

#[async_trait]
impl SourceWriter for LocalWriter {
    async fn write(&mut self, chunk: &[u8]) -> Result<(), IngestionError> {
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| IngestionError::SourceStore {
                message: "the upload is already closed".to_owned(),
                source: None,
            })?;
        file.write_all(chunk)
            .await
            .map_err(|err| IngestionError::storage("writing the upload failed", err))
    }

    async fn commit(mut self: Box<Self>) -> Result<(), IngestionError> {
        if let Some(mut file) = self.file.take() {
            file.flush()
                .await
                .map_err(|err| IngestionError::storage("flushing the upload failed", err))?;
            file.sync_all()
                .await
                .map_err(|err| IngestionError::storage("syncing the upload failed", err))?;
        }
        tokio::fs::rename(&self.temp, &self.target)
            .await
            .map_err(|err| IngestionError::storage("finalising the upload failed", err))?;
        self.committed = true;
        Ok(())
    }
}

#[async_trait]
impl SourceStore for LocalSourceStore {
    fn storage_key(&self, id: DataSourceId) -> String {
        format!("sources/{id}/source")
    }

    async fn create(&self, id: DataSourceId) -> Result<Box<dyn SourceWriter>, IngestionError> {
        let dir = self.dir(id);
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|err| IngestionError::storage("cannot create the upload directory", err))?;
        let temp = dir.join(format!(".upload-{}.tmp", uuid::Uuid::now_v7().simple()));
        let file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .await
            .map_err(|err| IngestionError::storage("cannot create the upload file", err))?;
        Ok(Box::new(LocalWriter {
            file: Some(file),
            temp,
            target: dir.join("source"),
            committed: false,
        }))
    }

    async fn open(&self, id: DataSourceId) -> Result<Box<dyn Read + Send>, IngestionError> {
        let path = self.dir(id).join("source");
        let file = std::fs::File::open(path)
            .map_err(|err| IngestionError::storage("cannot open the source file", err))?;
        Ok(Box::new(BufReader::new(file)))
    }

    async fn delete(&self, id: DataSourceId) -> Result<(), IngestionError> {
        match tokio::fs::remove_dir_all(self.dir(id)).await {
            Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(IngestionError::storage(
                "cannot delete the source file",
                err,
            )),
            _ => Ok(()),
        }
    }
}

type Files = Arc<Mutex<HashMap<DataSourceId, Arc<Vec<u8>>>>>;

/// In-memory source files, for tests.
#[derive(Debug, Clone, Default)]
pub struct MemorySourceStore {
    files: Files,
}

impl MemorySourceStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Stores a file directly (test setup).
    pub fn insert(&self, id: DataSourceId, bytes: impl Into<Vec<u8>>) {
        self.files
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, Arc::new(bytes.into()));
    }

    /// Whether a committed file exists.
    #[must_use]
    pub fn contains(&self, id: DataSourceId) -> bool {
        self.files
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key(&id)
    }
}

struct MemoryWriter {
    files: Files,
    id: DataSourceId,
    buffer: Vec<u8>,
}

#[async_trait]
impl SourceWriter for MemoryWriter {
    async fn write(&mut self, chunk: &[u8]) -> Result<(), IngestionError> {
        self.buffer.extend_from_slice(chunk);
        Ok(())
    }

    async fn commit(self: Box<Self>) -> Result<(), IngestionError> {
        self.files
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(self.id, Arc::new(self.buffer));
        Ok(())
    }
}

#[async_trait]
impl SourceStore for MemorySourceStore {
    fn storage_key(&self, id: DataSourceId) -> String {
        format!("memory/{id}")
    }

    async fn create(&self, id: DataSourceId) -> Result<Box<dyn SourceWriter>, IngestionError> {
        Ok(Box::new(MemoryWriter {
            files: Arc::clone(&self.files),
            id,
            buffer: Vec::new(),
        }))
    }

    async fn open(&self, id: DataSourceId) -> Result<Box<dyn Read + Send>, IngestionError> {
        let bytes = self
            .files
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&id)
            .cloned()
            .ok_or_else(|| IngestionError::SourceStore {
                message: format!("source file of data source {id} is missing"),
                source: None,
            })?;
        Ok(Box::new(Cursor::new(bytes.as_ref().clone())))
    }

    async fn delete(&self, id: DataSourceId) -> Result<(), IngestionError> {
        self.files
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn read_all(store: &dyn SourceStore, id: DataSourceId) -> String {
        let mut text = String::new();
        store
            .open(id)
            .await
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        text
    }

    #[test]
    fn digest_matches_known_sha256() {
        let mut digest = UploadDigest::new();
        digest.update(b"ab");
        digest.update(b"c");
        assert_eq!(
            digest.finish(),
            (
                3,
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".to_owned()
            )
        );
    }

    #[tokio::test]
    async fn local_store_writes_atomically_and_deletes() {
        let temp = tempfile::tempdir().unwrap();
        let store = LocalSourceStore::new(temp.path()).await.unwrap();
        let id = DataSourceId::new();
        let mut writer = store.create(id).await.unwrap();
        writer.write(b"id,name\n").await.unwrap();
        // Not visible before commit.
        assert!(store.open(id).await.is_err());
        writer.write(b"C1,Ann\n").await.unwrap();
        writer.commit().await.unwrap();
        assert_eq!(read_all(&store, id).await, "id,name\nC1,Ann\n");
        let dir = temp.path().join("sources").join(id.to_string());
        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, ["source"], "no temporary files remain");
        assert_eq!(store.storage_key(id), format!("sources/{id}/source"));
        store.delete(id).await.unwrap();
        assert!(!dir.exists());
        store.delete(id).await.unwrap();
    }

    #[tokio::test]
    async fn abandoned_uploads_leave_nothing_behind() {
        let temp = tempfile::tempdir().unwrap();
        let store = LocalSourceStore::new(temp.path()).await.unwrap();
        let id = DataSourceId::new();
        let mut writer = store.create(id).await.unwrap();
        writer.write(b"partial").await.unwrap();
        drop(writer);
        let dir = temp.path().join("sources").join(id.to_string());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        assert!(store.open(id).await.is_err());
    }

    #[tokio::test]
    async fn files_are_keyed_by_id_not_filename() {
        let temp = tempfile::tempdir().unwrap();
        let store = LocalSourceStore::new(temp.path()).await.unwrap();
        // Two uploads that a user might both call "../customers.csv".
        let (a, b) = (DataSourceId::new(), DataSourceId::new());
        for (id, body) in [(a, "A"), (b, "B")] {
            let mut writer = store.create(id).await.unwrap();
            writer.write(body.as_bytes()).await.unwrap();
            writer.commit().await.unwrap();
        }
        assert_eq!(read_all(&store, a).await, "A");
        assert_eq!(read_all(&store, b).await, "B");
        let outside = temp.path().parent().unwrap().join("customers.csv");
        assert!(!outside.exists());
        let everything: Vec<PathBuf> = walk(temp.path());
        assert!(everything.iter().all(|path| path.starts_with(temp.path())));
    }

    fn walk(dir: &std::path::Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                out.extend(walk(&path));
            } else {
                out.push(path);
            }
        }
        out
    }

    #[tokio::test]
    async fn memory_store_round_trips() {
        let store = MemorySourceStore::new();
        let id = DataSourceId::new();
        let mut writer = store.create(id).await.unwrap();
        writer.write(b"x").await.unwrap();
        assert!(!store.contains(id));
        writer.commit().await.unwrap();
        assert_eq!(read_all(&store, id).await, "x");
        store.delete(id).await.unwrap();
        assert!(store.open(id).await.is_err());
    }
}
