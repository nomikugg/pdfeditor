mod error;
mod models;
mod pdf;
mod routes;
mod services;
mod storage;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;
use pdfium_render::prelude::*;
use tracing::info;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use crate::error::AppError;
use crate::storage::file_store::FileStore;

#[derive(Clone)]
pub struct AppState {
    pub pdfium: Arc<Pdfium>,
    pub store: Arc<FileStore>,
}

#[tokio::main]
async fn main() -> Result<(), AppError> {
    let env_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".env");
    let _ = dotenvy::from_path(&env_path);

    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "pdf_editor_backend=info,tower_http=info".to_string()),
        )
        .init();

    let pdfium_dll_path = std::env::var("PDFIUM_DLL_PATH").unwrap_or_else(|_| "pdfium.dll".to_string());
    let bindings = Pdfium::bind_to_library(&pdfium_dll_path)
        .map_err(|e| AppError::Pdfium(format!("No se pudo cargar {}: {e}", pdfium_dll_path)))?;
    let pdfium = Arc::new(Pdfium::new(bindings));

    let files_root = PathBuf::from("files");
    let store = Arc::new(FileStore::new(files_root).await?);
    store.cleanup_older_than(std::time::Duration::from_secs(60 * 60 * 24 * 7)).await?;

    let state = AppState { pdfium, store };

    let app = Router::new()
        .route("/health", get(routes::health::health))
        .route("/pdf/upload", post(routes::upload::upload_pdf))
        .route("/pdf/analyze", post(routes::analyze::analyze_pdf))
        .route("/pdf/apply", post(routes::apply::apply_pdf_operations))
        .route("/pdf/download", get(routes::download::download_pdf))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 8080));
    info!("Backend iniciado en http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
