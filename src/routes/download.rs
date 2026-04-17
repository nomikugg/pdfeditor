use axum::extract::{Query, State};
use axum::http::{header, HeaderValue};
use axum::response::Response;

use crate::error::AppError;
use crate::AppState;
use serde::Deserialize;
use tracing::info;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct DownloadQuery {
    #[serde(rename = "fileId")]
    pub file_id: Uuid,
}

pub async fn download_pdf(
    State(state): State<AppState>,
    Query(query): Query<DownloadQuery>,
) -> Result<Response, AppError> {
    let bytes = state.store.read_pdf_bytes(&query.file_id).await?;
    info!("PDF descargado. fileId={}", query.file_id);

    let filename = format!("{}.pdf", query.file_id);
    let content_disposition = format!("attachment; filename=\"{}\"", filename);

    let response = Response::builder()
        .status(200)
        .header(header::CONTENT_TYPE, HeaderValue::from_static("application/pdf"))
        .header(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_str(&content_disposition)
                .map_err(|e| AppError::Internal(format!("No se pudo construir header: {e}")))?,
        )
        .body(axum::body::Body::from(bytes))
        .map_err(|e| AppError::Internal(format!("No se pudo construir response: {e}")))?;

    Ok(response)
}