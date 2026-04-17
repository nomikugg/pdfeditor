use axum::extract::{Multipart, State};
use axum::Json;
use tracing::info;

use crate::error::AppError;
use crate::models::response::UploadResponse;
use crate::AppState;

const MAX_UPLOAD_BYTES: usize = 25 * 1024 * 1024;

pub async fn upload_pdf(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<UploadResponse>, AppError> {
    let mut pdf_bytes: Option<Vec<u8>> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("multipart invalido: {e}")))?
    {
        let name = field.name().unwrap_or_default().to_string();

        if name == "file" {
            let bytes = field
                .bytes()
                .await
                .map_err(|e| AppError::BadRequest(format!("no se pudo leer el archivo: {e}")))?;
            pdf_bytes = Some(bytes.to_vec());
            break;
        }
    }

    let bytes = pdf_bytes.ok_or_else(|| {
        AppError::BadRequest("Debe enviar un campo multipart llamado 'file'".to_string())
    })?;

    if !bytes.starts_with(b"%PDF") {
        return Err(AppError::BadRequest("El archivo no parece ser un PDF valido".to_string()));
    }

    if bytes.len() > MAX_UPLOAD_BYTES {
        return Err(AppError::BadRequest("El PDF supera el limite permitido de 25 MB".to_string()));
    }

    let file_id = state.store.save_new_pdf(&bytes).await?;
    info!("PDF subido correctamente. fileId={}", file_id);

    Ok(Json(UploadResponse { file_id }))
}
