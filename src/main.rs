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

fn startup_debug_enabled() -> bool {
    std::env::var("STARTUP_DEBUG")
        .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

fn startup_debug(message: &str) {
    if startup_debug_enabled() {
        eprintln!("[startup] {message}");
    }
}

#[derive(Clone)]
pub struct AppState {
    pub pdfium: Arc<Pdfium>,
    pub store: Arc<FileStore>,
}

#[tokio::main]
async fn main() -> Result<(), AppError> {
    startup_debug("main: start");
    let env_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".env");
    let _ = dotenvy::from_path(&env_path);
    startup_debug("dotenv loaded (if present)");

    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "pdf_editor_backend=info,tower_http=info".to_string()),
        )
        .init();
    startup_debug("tracing initialized");

    let pdfium_library_path = resolve_pdfium_library_path();
    startup_debug(&format!("pdfium path: {pdfium_library_path}"));
    let bindings = Pdfium::bind_to_library(&pdfium_library_path)
        .map_err(|e| AppError::Pdfium(format!("No se pudo cargar {}: {e}", pdfium_library_path)))?;
    startup_debug("pdfium bindings loaded");
    let pdfium = Arc::new(Pdfium::new(bindings));
    startup_debug("pdfium instance ready");

    let files_root = std::env::var("FILES_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("files"));
    startup_debug(&format!("files root: {}", files_root.display()));
    let store = Arc::new(FileStore::new(files_root).await?);
    startup_debug("file store ready");
    store.cleanup_older_than(std::time::Duration::from_secs(60 * 60 * 24 * 7)).await?;
    startup_debug("local cleanup (if applicable) done");

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
    startup_debug("router built");

    let port = std::env::var("PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(8080);
    let host = std::env::var("BIND_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let addr = format!("{}:{}", host, port)
        .parse::<SocketAddr>()
        .map_err(|e| AppError::Internal(format!("No se pudo resolver la direccion de escucha: {e}")))?;
    info!("Backend iniciado en http://{}", addr);
    startup_debug(&format!("binding listener at {addr}"));

    let listener = tokio::net::TcpListener::bind(addr).await?;
    startup_debug("listener bound; starting server");
    axum::serve(listener, app).await?;

    Ok(())
}

fn resolve_pdfium_library_path() -> String {
    if let Ok(path) = std::env::var("PDFIUM_LIBRARY_PATH") {
        return path;
    }

    if let Ok(path) = std::env::var("PDFIUM_DLL_PATH") {
        return path;
    }

    #[cfg(target_os = "windows")]
    {
        return "pdfium.dll".to_string();
    }

    #[cfg(target_os = "linux")]
    {
        return "libpdfium.so".to_string();
    }

    #[cfg(target_os = "macos")]
    {
        return "libpdfium.dylib".to_string();
    }

    #[allow(unreachable_code)]
    "pdfium.dll".to_string()
}
