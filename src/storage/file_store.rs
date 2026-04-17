use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use reqwest::Client;
use tokio::fs;
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug)]
pub struct FileStore {
    backend: StorageBackend,
}

#[derive(Debug)]
enum StorageBackend {
    Local {
        root: PathBuf,
    },
    Supabase {
        url: String,
        service_role_key: String,
        bucket: String,
        client: Client,
    },
}

impl FileStore {
    pub async fn new(root: PathBuf) -> Result<Self, AppError> {
        let supabase_url = std::env::var("SUPABASE_URL").ok().map(|v| v.trim().to_string());
        let supabase_service_key = std::env::var("SUPABASE_SERVICE_ROLE_KEY")
            .ok()
            .map(|v| v.trim().to_string());
        let supabase_bucket = std::env::var("SUPABASE_STORAGE_BUCKET")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "pdf-files".to_string());

        if let (Some(url), Some(service_role_key)) = (supabase_url, supabase_service_key) {
            if !url.is_empty() && !service_role_key.is_empty() {
                return Ok(Self {
                    backend: StorageBackend::Supabase {
                        url,
                        service_role_key,
                        bucket: supabase_bucket,
                        client: Client::new(),
                    },
                });
            }
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
            StorageBackend::Supabase {
                url,
                service_role_key,
                bucket,
                client,
            } => upload_pdf_to_supabase(client, url, service_role_key, bucket, id, bytes).await,
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
            StorageBackend::Supabase {
                url,
                service_role_key,
                bucket,
                client,
            } => download_pdf_from_supabase(client, url, service_role_key, bucket, id).await,
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

fn supabase_object_name(id: &Uuid) -> String {
    format!("{id}.pdf")
}

async fn upload_pdf_to_supabase(
    client: &Client,
    url: &str,
    service_role_key: &str,
    bucket: &str,
    id: &Uuid,
    bytes: &[u8],
) -> Result<(), AppError> {
    let base_url = url.trim_end_matches('/');
    let object_name = supabase_object_name(id);
    let endpoint = format!("{base_url}/storage/v1/object/{bucket}/{object_name}");

    let response = client
        .post(endpoint)
        .header("apikey", service_role_key)
        .header("Authorization", format!("Bearer {service_role_key}"))
        .header("x-upsert", "true")
        .header("Content-Type", "application/pdf")
        .body(bytes.to_vec())
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("No se pudo subir a Supabase Storage: {e}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let details = response.text().await.unwrap_or_default();
        return Err(AppError::Internal(format!(
            "No se pudo subir a Supabase Storage: {} {details}",
            status
        )));
    }

    Ok(())
}

async fn download_pdf_from_supabase(
    client: &Client,
    url: &str,
    service_role_key: &str,
    bucket: &str,
    id: &Uuid,
) -> Result<Vec<u8>, AppError> {
    let base_url = url.trim_end_matches('/');
    let object_name = supabase_object_name(id);
    let endpoint = format!("{base_url}/storage/v1/object/{bucket}/{object_name}");

    let response = client
        .get(endpoint)
        .header("apikey", service_role_key)
        .header("Authorization", format!("Bearer {service_role_key}"))
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("No se pudo descargar desde Supabase Storage: {e}")))?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(AppError::NotFound(format!("No existe fileId={id}")));
    }

    if !response.status().is_success() {
        let status = response.status();
        let details = response.text().await.unwrap_or_default();
        if status == reqwest::StatusCode::BAD_REQUEST && details.to_ascii_lowercase().contains("not found") {
            return Err(AppError::NotFound(format!("No existe fileId={id}")));
        }

        return Err(AppError::Internal(format!(
            "No se pudo descargar desde Supabase Storage: {} {details}",
            status
        )));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| AppError::Internal(format!("No se pudo leer bytes desde Supabase Storage: {e}")))?;

    Ok(bytes.to_vec())
}
