use axum::extract::State;
use axum::Json;

use crate::error::AppError;
use crate::models::request::AnalyzeRequest;
use crate::models::response::AnalyzeResponse;
use crate::services::pdf_service;
use crate::AppState;

pub async fn analyze_pdf(
    State(state): State<AppState>,
    Json(payload): Json<AnalyzeRequest>,
) -> Result<Json<AnalyzeResponse>, AppError> {
    let response = pdf_service::analyze_pdf(&state.pdfium, &state.store, payload.file_id).await?;
    Ok(Json(response))
}
