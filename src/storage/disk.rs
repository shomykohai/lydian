use std::path::{Component, Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use axum::body::Bytes;
use futures::{Stream, StreamExt};
use tokio::fs::{File, copy, create_dir_all, read_dir, remove_file, rename, try_exists};
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;

use super::keys::StorageKey;
use crate::utils::mime::MediaType;

pub struct LocalFile {
    pub path: PathBuf,
    pub stream: ReaderStream<File>,
}

#[derive(Clone)]
pub struct DiskDriver {
    root_path: PathBuf,
}

impl DiskDriver {
    pub async fn new(path: &str) -> Result<Self> {
        let root_path = PathBuf::from(path);
        if !root_path.exists() {
            create_dir_all(&root_path).await?;
        }
        Ok(Self { root_path })
    }

    pub fn resolve_path(&self, key: &str) -> PathBuf {
        let mut safe_path = self.root_path.clone();

        for component in Path::new(key).components() {
            match component {
                Component::Normal(seg) => {
                    safe_path.push(seg);
                }
                Component::ParentDir
                    if safe_path.starts_with(&self.root_path) && safe_path != self.root_path =>
                {
                    safe_path.pop();
                }
                _ => {}
            }
        }

        safe_path
    }

    pub async fn resolve_path_prefix(&self, key: &str) -> Option<PathBuf> {
        let target_path = self.resolve_path(key);

        for suffix in MediaType::extensions() {
            let mut candidate = target_path.clone().into_os_string();
            candidate.push(".");
            candidate.push(suffix);

            let candidate = PathBuf::from(candidate);

            if try_exists(&candidate).await.unwrap_or(false) {
                return Some(candidate);
            }
        }

        self.scan_for_prefix(&target_path).await
    }

    async fn scan_for_prefix(&self, target_path: &Path) -> Option<PathBuf> {
        let parent_dir = target_path.parent()?;
        let target_name = target_path.file_name()?.to_str()?;

        let mut entries = read_dir(parent_dir).await.ok()?;

        while let Some(entry) = entries.next_entry().await.ok()? {
            let Some(file_name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };

            if !file_name.strip_prefix(target_name).is_some_and(|rest| rest.starts_with('.')) {
                continue;
            }

            if entry.file_type().await.is_ok_and(|kind| kind.is_file()) {
                return Some(entry.path());
            }
        }

        None
    }

    pub async fn exists_by_prefix(&self, key: &str) -> bool {
        self.resolve_path_prefix(key).await.is_some()
    }

    pub async fn exists(&self, key: &str) -> bool {
        try_exists(self.resolve_path(key)).await.unwrap_or(false)
    }

    pub async fn get_file(&self, key: &str) -> Result<File> {
        let file = File::open(self.resolve_path(key)).await?;
        Ok(file)
    }

    pub async fn get_file_by_prefix(&self, key: &str) -> Result<File> {
        if let Some(path) = self.resolve_path_prefix(key).await {
            let file = File::open(path).await?;
            return Ok(file);
        }

        bail!("No cached file found matching key prefix: {}", key);
    }

    pub async fn get_extension_by_prefix(&self, key: &str) -> Option<String> {
        if let Some(path) = self.resolve_path_prefix(key).await {
            return path.extension().map(|ext| ext.to_string_lossy().to_string());
        }

        None
    }

    pub async fn read_stream(&self, key: &str) -> Result<LocalFile> {
        let file = File::open(self.resolve_path(key)).await?;
        Ok(LocalFile { path: self.resolve_path(key), stream: ReaderStream::new(file) })
    }

    pub async fn read_stream_by_prefix(&self, key: &str) -> Result<LocalFile> {
        if let Some(path) = self.resolve_path_prefix(key).await {
            let file = File::open(&path).await?;
            return Ok(LocalFile { path, stream: ReaderStream::new(file) });
        }

        bail!("No cached file found matching key prefix: {}", key);
    }

    pub async fn write_stream<S>(&self, key: &str, mut upstream: S) -> Result<()>
    where
        S: Stream<Item = Bytes> + Send + Unpin + 'static,
    {
        let path = Path::new(key);
        let parent = path.parent().unwrap_or_else(|| Path::new(""));
        let name = path.file_name().ok_or_else(|| anyhow::anyhow!("Invalid key"))?;
        let tmp_key = parent.join("tmp").join(name).to_string_lossy().to_string();

        let mut file = self.create_file(&tmp_key).await?;
        let disk = self.clone();
        let target_key = key.to_string();

        tokio::spawn(async move {
            while let Some(chunk) = upstream.next().await {
                if file.write_all(&chunk).await.is_err() {
                    return;
                }
            }
            if file.flush().await.is_ok() {
                let _ = disk.rename_file(&tmp_key, &target_key).await;
            }
        });

        Ok(())
    }

    pub async fn save_stream<S, E>(&self, key: &str, mut stream: S) -> Result<()>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin,
        E: Into<anyhow::Error>,
    {
        let path = Path::new(key);
        let parent = path.parent().unwrap_or_else(|| Path::new(""));
        let name = path.file_name().ok_or_else(|| anyhow::anyhow!("Invalid key"))?;
        let tmp_key = parent.join("tmp").join(name).to_string_lossy().to_string();

        let mut file = self.create_file(&tmp_key).await?;

        while let Some(chunk_res) = stream.next().await {
            let chunk = chunk_res.map_err(|e| e.into())?;
            file.write_all(&chunk).await?;
        }

        file.flush().await?;
        drop(file);

        self.rename_file(&tmp_key, key).await?;
        Ok(())
    }

    pub async fn create_file(&self, key: &str) -> Result<File> {
        let target_path = self.resolve_path(key);
        if let Some(parent) = target_path.parent()
            && !parent.exists()
        {
            create_dir_all(parent).await?;
        }
        Ok(File::create(target_path).await?)
    }

    pub async fn rename_file(&self, from_key: &str, to_key: &str) -> Result<()> {
        let from_path = self.resolve_path(from_key);
        let to_path = self.resolve_path(to_key);
        if let Some(parent) = to_path.parent()
            && !parent.exists()
        {
            create_dir_all(parent).await?;
        }
        rename(from_path, to_path).await?;
        Ok(())
    }

    pub async fn move_file(&self, from_key: &str, to: &str) -> Result<()> {
        let from_path = self.resolve_path(from_key);
        let to_path = PathBuf::from(to);

        if let Some(parent) = to_path.parent()
            && !parent.exists()
        {
            create_dir_all(parent).await?;
        }

        copy(from_path, to_path).await?;
        remove_file(self.resolve_path(from_key)).await?;
        Ok(())
    }

    pub async fn move_file_by_prefix(&self, from_key: &str, to: &str) -> Result<PathBuf> {
        // TODO: Just... make it decent
        let from_path = self
            .resolve_path_prefix(from_key)
            .await
            .ok_or_else(|| anyhow!(std::io::ErrorKind::NotFound))?;
        let to_path = PathBuf::from(format!(
            "{}{}",
            to,
            from_path
                .extension()
                .map(|ext| format!(".{}", ext.to_string_lossy()))
                .unwrap_or_default()
        ));

        if let Some(parent) = to_path.parent()
            && !parent.exists()
        {
            create_dir_all(parent).await?;
        }

        copy(&from_path, &to_path).await?;
        remove_file(from_path).await?;
        Ok(to_path)
    }

    pub async fn delete_file(&self, key: &str) -> Result<()> {
        remove_file(self.resolve_path(key)).await?;
        Ok(())
    }

    pub async fn has_key(&self, key: &StorageKey) -> bool {
        self.exists(&key.to_file_name()).await
    }

    pub fn tmp_key_for(file_name: &str) -> Result<String> {
        let path = Path::new(file_name);
        let parent = path.parent().unwrap_or_else(|| Path::new(""));
        let name = path.file_name().ok_or_else(|| anyhow::anyhow!("Invalid file name"))?;

        Ok(parent.join("tmp").join(name).to_string_lossy().to_string())
    }

    pub async fn create_temp_for_name(&self, file_name: &str) -> Result<File> {
        self.create_file(&Self::tmp_key_for(file_name)?).await
    }

    pub async fn commit_temp_for_name(&self, file_name: &str) -> Result<()> {
        let tmp_key = Self::tmp_key_for(file_name)?;

        if let Ok(file) = File::open(self.resolve_path(&tmp_key)).await {
            file.sync_all().await?;
        }

        self.rename_file(&tmp_key, file_name).await
    }

    pub async fn create_temp_for_key(&self, key: &StorageKey) -> Result<File> {
        self.create_temp_for_name(&key.to_file_name()).await
    }

    pub async fn commit_temp_for_key(&self, key: &StorageKey) -> Result<()> {
        self.commit_temp_for_name(&key.to_file_name()).await
    }
}
