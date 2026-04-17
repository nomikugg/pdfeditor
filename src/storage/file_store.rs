use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use reqwest::Client;
use serde::Deserialize;
use tokio::fs;
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug)]
pub struct FileStore {
    backend: StorageBackend,
}

#[derive(Debug)]
enum StorageBackend {
    Local { root: PathBuf },
    Gcs { bucket: String, client: Client },
}

#[derive(Debug, Deserialize)]
struct MetadataTokenResponse {
    access_token: String,
}

impl FileStore {
    pub async fn new(root: PathBuf) -> Result<Self, AppError> {
        if let Ok(bucket) = std::env::var("GCS_BUCKET_NAME").or_else(|_| std::env::var("GCS_BUCKET")) {
            return Ok(Self {
                backend: StorageBackend::Gcs {
                    bucket,
                    client: Client::new(),
                },
            });
        }

        if !root.exists() {
            fs::create_dir_all(&root).await?;
        }

        Ok(Self {
            backend: StorageBackend::Local { root },
        })
    }

    pub async fn save_new_pdf(&self, bytes: &[u8]) -> Result<Uuid, AppError> {
        let id = Uuid::new_v4();
        self.save_pdf_bytes(&id, bytes).await?;
        Ok(id)
    }

    pub async fn save_pdf_bytes(&self, id: &Uuid, bytes: &[u8]) -> Result<(), AppError> {
        match &self.backend {
            StorageBackend::Local { root } => {
                let path = root.join(format!("{id}.pdf"));
                fs::write(path, bytes).await?;
                Ok(())
            }
            StorageBackend::Gcs { bucket, client } => {
                let token = resolve_gcs_access_token(client).await?;
                upload_pdf_to_gcs(client, bucket, id, bytes, &token).await
            }
        }
    }

    pub async fn read_pdf_bytes(&self, id: &Uuid) -> Result<Vec<u8>, AppError> {
        match &self.backend {
            StorageBackend::Local { root } => {
                let path = root.join(format!("{id}.pdf"));
                if !path.exists() {
                    return Err(AppError::NotFound(format!("No existe fileId={id}")));
                }

                Ok(fs::read(path).await?)
            }
            StorageBackend::Gcs { bucket, client } => {
                let token = resolve_gcs_access_token(client).await?;
                download_pdf_from_gcs(client, bucket, id, &token).await
            }
        }
    }

    pub async fn cleanup_older_than(&self, max_age: Duration) -> Result<(), AppError> {
        let StorageBackend::Local { root } = &self.backend else {
            return Ok(());
        };

        let mut entries = fs::read_dir(root).await?;

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
}

async fn resolve_gcs_access_token(client: &Client) -> Result<String, AppError> {
    if let Ok(token) = std::env::var("GCS_ACCESS_TOKEN").or_else(|_| std::env::var("GOOGLE_OAUTH_ACCESS_TOKEN")) {
        let trimmed = token.trim().to_string();
        if !trimmed.is_empty() {
            return Ok(trimmed);
        }
    }

    let response = client
        .get("http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token")
        .header("Metadata-Flavor", "Google")
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("No se pudo obtener token de GCP: {e}")))?;

    if !response.status().is_success() {
        return Err(AppError::Internal(format!("No se pudo obtener token de GCP: {}", response.status())));
    }

    let payload = response
        .json::<MetadataTokenResponse>()
        .await
        .map_err(|e| AppError::Internal(format!("No se pudo leer token de GCP: {e}")))?;

    Ok(payload.access_token)
}

async fn upload_pdf_to_gcs(
    client: &Client,
    bucket: &str,
    id: &Uuid,
    bytes: &[u8],
    token: &str,
) -> Result<(), AppError> {
    let object_name = format!("{id}.pdf");
    let url = format!(
        "https://storage.googleapis.com/upload/storage/v1/b/{bucket}/o?uploadType=media&name={object_name}"
    );

    let response = client
        .post(url)
        .bearer_auth(token)
        .header("Content-Type", "application/pdf")
        .body(bytes.to_vec())
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("No se pudo subir a GCS: {e}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let details = response.text().await.unwrap_or_default();
        return Err(AppError::Internal(format!("No se pudo subir a GCS: {} {details}", status)));
    }

    Ok(())
}

async fn download_pdf_from_gcs(
    client: &Client,
    bucket: &str,
    id: &Uuid,
    token: &str,
) -> Result<Vec<u8>, AppError> {
    let object_name = format!("{id}.pdf");
    let url = format!(
        "https://storage.googleapis.com/storage/v1/b/{bucket}/o/{object_name}?alt=media"
    );

    let response = client
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("No se pudo descargar desde GCS: {e}")))?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(AppError::NotFound(format!("No existe fileId={id}")));
    }

    if !response.status().is_success() {
        let status = response.status();
        let details = response.text().await.unwrap_or_default();
        return Err(AppError::Internal(format!("No se pudo descargar desde GCS: {} {details}", status)));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| AppError::Internal(format!("No se pudo leer bytes desde GCS: {e}")))?;

    Ok(bytes.to_vec())
}
