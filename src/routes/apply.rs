use axum::extract::State;
use axum::Json;

use crate::error::AppError;
use crate::models::request::ApplyRequest;
use crate::models::response::ApplyResponse;
use crate::services::pdf_service;
use crate::AppState;

pub async fn apply_pdf_operations(
    State(state): State<AppState>,
    Json(payload): Json<ApplyRequest>,
) -> Result<Json<ApplyResponse>, AppError> {
    let response =
        pdf_service::apply_operations(&state.pdfium, &state.store, payload.file_id, payload.operations)
            .await?;
    Ok(Json(response))
}
