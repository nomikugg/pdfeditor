use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadResponse {
    pub file_id: Uuid,
}

#[derive(Debug, Serialize)]
pub struct AnalyzeResponse {
    pub pages: Vec<PageAnalysis>,
}

#[derive(Debug, Serialize)]
pub struct PageAnalysis {
    pub page: usize,
    pub texts: Vec<TextBox>,
    pub images: Vec<ImageBox>,
}

#[derive(Debug, Serialize)]
pub struct TextBox {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    #[serde(rename = "fontName", skip_serializing_if = "Option::is_none")]
    pub font_name: Option<String>,
    #[serde(rename = "fontFamily", skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    #[serde(rename = "fontSize", skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f32>,
}

#[derive(Debug, Serialize)]
pub struct ImageBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResponse {
    pub file_id: Uuid,
}
