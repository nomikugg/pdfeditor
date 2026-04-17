use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use tokio::fs;
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug)]
pub struct FileStore {
    root: PathBuf,
}

impl FileStore {
    pub async fn new(root: PathBuf) -> Result<Self, AppError> {
        if !root.exists() {
            fs::create_dir_all(&root).await?;
        }

        Ok(Self { root })
    }

    pub async fn save_new_pdf(&self, bytes: &[u8]) -> Result<Uuid, AppError> {
        let id = Uuid::new_v4();
        let path = self.path_for_uuid(&id);
        fs::write(path, bytes).await?;
        Ok(id)
    }

    pub async fn cleanup_older_than(&self, max_age: Duration) -> Result<(), AppError> {
        let mut entries = fs::read_dir(&self.root).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();

            if path.extension().and_then(|ext| ext.to_str()) != Some("pdf") {
                continue;
            }

            let metadata = entry.metadata().await?;
            let modified = metadata.modified().unwrap_or(SystemTime::now());

            if SystemTime::now()
                .duration_since(modified)
                .unwrap_or_default()
                > max_age
            {
                let _ = fs::remove_file(path).await;
            }
        }

        Ok(())
    }

    pub fn path_for(&self, id: &Uuid) -> Option<PathBuf> {
        let path = self.path_for_uuid(id);
        if Path::new(&path).exists() {
            Some(path)
        } else {
            None
        }
    }

    pub fn path_for_uuid(&self, id: &Uuid) -> PathBuf {
        self.root.join(format!("{id}.pdf"))
    }
}
