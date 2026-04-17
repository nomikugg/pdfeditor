use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use lopdf::content::Content;
use lopdf::{Document as LoDocument, Object, StringFormat};
use pdfium_render::prelude::*;
use tracing::{info, warn};
use ttf_parser::Face;
use uuid::Uuid;

use crate::error::AppError;
use crate::models::request::{OperationPayload, OperationType};
use crate::models::response::{AnalyzeResponse, ApplyResponse, ImageBox, PageAnalysis, TextBox};
use crate::pdf::font::ensure_unicode_overlay_fonts;
use crate::pdf::renderer::{append_unicode_text_operations, DrawTextOp};
use crate::storage::file_store::FileStore;

pub async fn analyze_pdf(
    pdfium: &Pdfium,
    store: &Arc<FileStore>,
    file_id: Uuid,
) -> Result<AnalyzeResponse, AppError> {
    let input_path = store
        .path_for(&file_id)
        .ok_or_else(|| AppError::NotFound(format!("No existe fileId={file_id}")))?;

    let document = pdfium
        .load_pdf_from_file(&input_path, None)
        .map_err(|e| AppError::Pdfium(format!("No se pudo abrir el PDF: {e}")))?;

    let mut pages_output = Vec::new();

    for page_index in 0..document.pages().len() {
        let page = document
            .pages()
            .get(page_index)
            .map_err(|e| AppError::Pdfium(format!("No se pudo leer la pagina {page_index}: {e}")))?;

        let mut texts = Vec::new();
        let mut images = Vec::new();

        for object in page.objects().iter() {
            if let Some(text_object) = object.as_text_object() {
                let raw_text = text_object.text();
                let text = raw_text.trim().to_string();

                if text.is_empty() {
                    continue;
                }

                let quad = object
                    .bounds()
                    .map_err(|e| AppError::Pdfium(format!("No se pudo obtener bounds: {e}")))?;
                let rect = quad.to_rect();
                let font = text_object.font();
                let font_name = font.name();
                let font_family = font.family();
                let font_size = text_object.unscaled_font_size().value;

                texts.push(TextBox {
                    text,
                    x: rect.left().value,
                    y: rect.bottom().value,
                    width: rect.width().value,
                    height: rect.height().value,
                    font_name: if font_name.trim().is_empty() {
                        None
                    } else {
                        Some(font_name)
                    },
                    font_family: if font_family.trim().is_empty() {
                        None
                    } else {
                        Some(font_family)
                    },
                    font_size: Some(font_size),
                });
                continue;
            }

            if object.as_image_object().is_some() {
                let quad = object
                    .bounds()
                    .map_err(|e| AppError::Pdfium(format!("No se pudo obtener bounds de imagen: {e}")))?;
                let rect = quad.to_rect();

                images.push(ImageBox {
                    x: rect.left().value,
                    y: rect.bottom().value,
                    width: rect.width().value,
                    height: rect.height().value,
                });
            }
        }

        pages_output.push(PageAnalysis {
            page: page_index as usize,
            texts,
            images,
        });
    }

    Ok(AnalyzeResponse { pages: pages_output })
}

pub async fn apply_operations(
    _pdfium: &Pdfium,
    store: &Arc<FileStore>,
    file_id: Uuid,
    operations: Vec<OperationPayload>,
) -> Result<ApplyResponse, AppError> {
    info!("Recibi apply_operations: fileId={}, ops={}", file_id, operations.len());

    let input_path = store
        .path_for(&file_id)
        .ok_or_else(|| AppError::NotFound(format!("No existe fileId={file_id}")))?;

    let mut document =
        LoDocument::load(&input_path).map_err(|e| AppError::Pdfium(format!("No se pudo abrir el PDF: {e}")))?;

    let pages = document.get_pages();
    let mut batch_used_utf16 = BTreeSet::new();
    for op in &operations {
        for unit in op.new_text.encode_utf16() {
            batch_used_utf16.insert(unit);
        }
    }

    for op in operations {
        info!(
            "Operación recibida: page={} newText='{}' targetText={:?} x={:?} y={:?} width={:?} height={:?}",
            op.page,
            op.new_text,
            op.target_text,
            op.x,
            op.y,
            op.width,
            op.height
        );
        match op.op_type {
            OperationType::Replace => {
                apply_replace_operation(&mut document, &pages, op, &batch_used_utf16)?;
            }
        }
    }

    let new_file_id = Uuid::new_v4();
    let out_path = store.path_for_uuid(&new_file_id);

    document
        .save(&out_path)
        .map_err(|e| AppError::Pdfium(format!("No se pudo guardar el PDF de salida: {e}")))?;

    info!("PDF modificado guardado. oldFileId={}, newFileId={}", file_id, new_file_id);

    Ok(ApplyResponse {
        file_id: new_file_id,
    })
}

fn apply_replace_operation(
    document: &mut LoDocument,
    pages: &std::collections::BTreeMap<u32, lopdf::ObjectId>,
    op: OperationPayload,
    batch_used_utf16: &BTreeSet<u16>,
) -> Result<(), AppError> {
    let overlay_regular_font: &[u8] = b"FCOHelveticaRegular";
    let overlay_bold_font: &[u8] = b"FCOHelveticaBold";
    let mut current_font: Vec<u8> = b"".to_vec();
    let page_number = (op.page + 1) as u32;
    let page_id = pages
        .get(&page_number)
        .copied()
        .ok_or_else(|| AppError::BadRequest(format!("Pagina invalida {}", op.page)))?;

    let content_data = document
        .get_page_content(page_id)
        .map_err(|e| AppError::Pdfium(format!("No se pudo leer content stream: {e}")))?;

    let mut content = Content::decode(&content_data)
        .map_err(|e| AppError::Pdfium(format!("No se pudo decodificar content stream: {e}")))?;

    let bold_font_keys = collect_bold_font_keys(document, page_id)?;
    let mut state = TextState::default();
    let mut graphics_state = GraphicsState::default();
    let mut replacements = 0usize;
    let mut overlays: Vec<DrawTextOp> = Vec::new();
    let page_width = get_page_width(document, page_id);

    for operation in content.operations.iter_mut() {
        match operation.operator.as_str() {
            "rg" => {
                if let (Some(r), Some(g), Some(b)) = (
                    operation.operands.first().and_then(as_f32),
                    operation.operands.get(1).and_then(as_f32),
                    operation.operands.get(2).and_then(as_f32),
                ) {
                    graphics_state.fill_rgb = Some((r, g, b));
                    graphics_state.fill_gray = None;
                }
            }
            "RG" => {
                if let (Some(r), Some(g), Some(b)) = (
                    operation.operands.first().and_then(as_f32),
                    operation.operands.get(1).and_then(as_f32),
                    operation.operands.get(2).and_then(as_f32),
                ) {
                    graphics_state.stroke_rgb = Some((r, g, b));
                }
            }
            "g" => {
                if let Some(gray) = operation.operands.first().and_then(as_f32) {
                    graphics_state.fill_gray = Some(gray);
                    graphics_state.fill_rgb = None;
                }
            }
            "G" => {
                if let Some(gray) = operation.operands.first().and_then(as_f32) {
                    graphics_state.stroke_rgb = Some((gray, gray, gray));
                }
            }
            "BT" => {
                state = TextState::default();
            }
            "Tf" => {
                if let Some(Object::Name(font_key)) = operation.operands.first() {
                    current_font = font_key.clone();
                    state.is_bold = bold_font_keys.contains(font_key.as_slice());
                }
                if let Some(font_size) = operation.operands.get(1).and_then(as_f32) {
                    state.font_size = font_size.max(1.0);
                    state.font_size_scaled = state.font_size;
                    graphics_state.font_size = state.font_size;
                }
            }
            "TL" => {
                if let Some(leading) = operation.operands.first().and_then(as_f32) {
                    state.leading = leading;
                }
            }
            "Td" => {
                if let (Some(tx), Some(ty)) = (
                    operation.operands.first().and_then(as_f32),
                    operation.operands.get(1).and_then(as_f32),
                ) {
                    state.x += tx;
                    state.y += ty;
                }
            }
            "TD" => {
                if let (Some(tx), Some(ty)) = (
                    operation.operands.first().and_then(as_f32),
                    operation.operands.get(1).and_then(as_f32),
                ) {
                    state.x += tx;
                    state.y += ty;
                    state.leading = -ty;
                }
            }
            "Tm" => {
                if let (Some(a), Some(b), Some(c), Some(d), Some(e), Some(f)) = (
                    operation.operands.first().and_then(as_f32),
                    operation.operands.get(1).and_then(as_f32),
                    operation.operands.get(2).and_then(as_f32),
                    operation.operands.get(3).and_then(as_f32),
                    operation.operands.get(4).and_then(as_f32),
                    operation.operands.get(5).and_then(as_f32),
                ) {
                    state.text_matrix = [a, b, c, d, e, f];
                    state.x = e;
                    state.y = f;
                    state.font_size_scaled = (state.font_size * d.abs()).max(1.0);
                    info!(
                        "Escala detectada: d={:.3}, tamaño base={:.3}, tamaño final={:.3}",
                        d,
                        state.font_size,
                        state.font_size_scaled
                    );
                }
            }
            "T*" => {
                advance_to_next_line(&mut state);
            }
            "Tj" => {
                if replacements > 0 {
                    continue;
                }
                if let Some(first) = operation.operands.get_mut(0) {
                    let overlay_font = if state.is_bold { overlay_bold_font } else { overlay_regular_font };
                    replacements += replace_text_object(
                        first,
                        &state,
                        &graphics_state,
                        &op,
                        page_width,
                        &mut current_font,
                        overlay_font,
                        &mut overlays,
                    );
                }
            }
            "'" => {
                if replacements > 0 {
                    continue;
                }
                advance_to_next_line(&mut state);
                if let Some(first) = operation.operands.get_mut(0) {
                    let overlay_font = if state.is_bold { overlay_bold_font } else { overlay_regular_font };
                    replacements += replace_text_object(
                        first,
                        &state,
                        &graphics_state,
                        &op,
                        page_width,
                        &mut current_font,
                        overlay_font,
                        &mut overlays,
                    );
                }
            }
            "\"" => {
                if replacements > 0 {
                    continue;
                }
                advance_to_next_line(&mut state);
                if let Some(third) = operation.operands.get_mut(2) {
                    let overlay_font = if state.is_bold { overlay_bold_font } else { overlay_regular_font };
                    replacements += replace_text_object(
                        third,
                        &state,
                        &graphics_state,
                        &op,
                        page_width,
                        &mut current_font,
                        overlay_font,
                        &mut overlays,
                    );
                }
            }
            "TJ" => {
                if replacements > 0 {
                    continue;
                }
                if let Some(first) = operation.operands.get_mut(0) {
                    let overlay_font = if state.is_bold { overlay_bold_font } else { overlay_regular_font };
                    replacements += replace_text_in_array(
                        first,
                        &state,
                        &graphics_state,
                        &op,
                        page_width,
                        &mut current_font,
                        overlay_font,
                        &mut overlays,
                    );
                }
            }
            _ => {}
        }
    }

    if replacements == 0 {
        warn!("No hubo match para reemplazo en page={} con payload={:?}", op.page, op);
        return Ok(());
    }

    if !overlays.is_empty() {
        for (idx, overlay) in overlays.iter().enumerate() {
            let font_name = if overlay.font_name.is_empty() {
                "<default>".to_string()
            } else {
                String::from_utf8_lossy(&overlay.font_name).to_string()
            };
            let color_str = overlay
                .color_rgb
                .map(|(r, g, b)| format!("({r:.3}, {g:.3}, {b:.3})"))
                .unwrap_or_else(|| "<none>".to_string());

            info!(
                "Overlay final [{}] page={} x={:.2} y={:.2} size={:.2} bold={} font={} color={} text='{}'",
                idx,
                op.page,
                overlay.x,
                overlay.y,
                overlay.size,
                overlay.is_bold,
                font_name,
                color_str,
                overlay.text
            );
        }

        let overlay_fonts = ensure_unicode_overlay_fonts(document, page_id, batch_used_utf16)?;
        append_unicode_text_operations(&mut content, &overlay_fonts, &overlays);
    }

    let encoded = content
        .encode()
        .map_err(|e| AppError::Pdfium(format!("No se pudo codificar content stream: {e}")))?;

    document
        .change_page_content(page_id, encoded)
        .map_err(|e| AppError::Pdfium(format!("No se pudo actualizar content stream: {e}")))?;

    info!(
        "Reemplazo exitoso via content stream por coordenadas en pagina {} ({} ocurrencias)",
        op.page, replacements
    );

    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct TextState {
    x: f32,
    y: f32,
    font_size: f32,
    font_size_scaled: f32,
    leading: f32,
    is_bold: bool,
    text_matrix: [f32; 6],
}

#[derive(Debug, Clone, Copy)]
struct GraphicsState {
    fill_rgb: Option<(f32, f32, f32)>,
    stroke_rgb: Option<(f32, f32, f32)>,
    fill_gray: Option<f32>,
    font_size: f32,
}

impl Default for TextState {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            font_size: 12.0,
            font_size_scaled: 12.0,
            leading: 14.0,
            is_bold: false,
            text_matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        }
    }
}

impl Default for GraphicsState {
    fn default() -> Self {
        Self {
            fill_rgb: Some((0.0, 0.0, 0.0)),
            stroke_rgb: None,
            fill_gray: None,
            font_size: 12.0,
        }
    }
}

fn advance_to_next_line(state: &mut TextState) {
    state.x = 0.0;
    state.y -= state.leading;
}

fn replace_text_object(
    object: &mut Object,
    state: &TextState,
    graphics_state: &GraphicsState,
    op: &OperationPayload,
    page_width: Option<f32>,
    current_font: &mut Vec<u8>, // 👈 NUEVO
    overlay_font: &[u8],        // 👈 NUEVO
    overlays: &mut Vec<DrawTextOp>,
) -> usize {
    if let Object::String(bytes, format) = object {
        let preview = decode_pdf_string(bytes);

        if !is_match_for_operation(state, op, &preview) {
            return 0;
        }

        info!(
            "Match texto simple: state=({}, {}) original='{}' new='{}'",
            state.x,
            state.y,
            preview,
            op.new_text
        );
        info!("Original bytes: {:02X?}", bytes);
        info!("Decoded preview: '{}'", preview);
        info!("Each char as hex:");
        for ch in preview.chars() {
            info!("  '{}' = U+{:04X}", ch, ch as u32);
        }

        let safe_new_text = op.new_text.trim();
        if safe_new_text.is_empty() {
            return 0;
        }

        *current_font = overlay_font.to_vec();
        let requested_size = op
            .font_size
            .unwrap_or(state.font_size_scaled)
            .max(1.0);
        let requested_bold = op.bold.unwrap_or(state.is_bold);
        let requested_font = resolve_overlay_font_name(op, requested_bold);
        let original_color = graphics_state
            .fill_rgb
            .or_else(|| graphics_state.fill_gray.map(|gray| (gray, gray, gray)))
            .or(Some((0.0, 0.0, 0.0)));
        let requested_color = parse_rgb_color(op.color.as_deref()).or(original_color);
        let mut requested_x = state.x;
        if is_plate_operation(op) {
            if let Some(centered_x) = center_text_on_page_if_needed(page_width, op, safe_new_text, requested_bold, requested_size) {
                requested_x = centered_x;
            }
        }
        info!(
            "📏 Tamaño de fuente - Base: {:.3}, Escalado: {:.3}, Matriz: [{:.3}, {:.3}, {:.3}, {:.3}, {:.3}, {:.3}]",
            state.font_size,
            state.font_size_scaled,
            state.text_matrix[0],
            state.text_matrix[1],
            state.text_matrix[2],
            state.text_matrix[3],
            state.text_matrix[4],
            state.text_matrix[5]
        );
        bytes.clear();
        *format = StringFormat::Literal;
        overlays.push(DrawTextOp {
            x: requested_x,
            y: state.y,
            size: requested_size,
            is_bold: requested_bold,
            font_name: requested_font,
            color_rgb: requested_color,
            text: safe_new_text.to_string(),
        });
        return 1;
    }

    0
}

fn replace_text_in_array(
    object: &mut Object,
    state: &TextState,
    graphics_state: &GraphicsState,
    op: &OperationPayload,
    page_width: Option<f32>,
    current_font: &mut Vec<u8>,
    overlay_font: &[u8],
    overlays: &mut Vec<DrawTextOp>,
) -> usize {
    if let Object::Array(items) = object {
        let mut joined = String::new();

        for item in items.iter() {
            if let Object::String(bytes, _) = item {
                joined.push_str(&decode_pdf_string(bytes));
            }
        }

        if joined.is_empty() || !is_match_for_operation(state, op, &joined) {
            return 0;
        }

        info!(
            "Match TJ: state=({}, {}) original='{}' new='{}'",
            state.x,
            state.y,
            joined,
            op.new_text
        );
        info!("TJ decoded joined preview: '{}'", joined);
        info!("TJ each char as hex:");
        for ch in joined.chars() {
            info!("  '{}' = U+{:04X}", ch, ch as u32);
        }
        info!("TJ original fragment bytes:");
        for (idx, item) in items.iter().enumerate() {
            if let Object::String(raw_bytes, _) = item {
                info!("  fragment[{}] bytes: {:02X?}", idx, raw_bytes);
            }
        }

        let safe_new_text = op.new_text.trim();
        if safe_new_text.is_empty() {
            return 0;
        }

        for item in items.iter_mut() {
            if let Object::String(rem_bytes, rem_format) = item {
                rem_bytes.clear();
                *rem_format = StringFormat::Literal;
            }
        }

        *current_font = overlay_font.to_vec();
        let requested_size = op
            .font_size
            .unwrap_or(state.font_size_scaled)
            .max(1.0);
        let requested_bold = op.bold.unwrap_or(state.is_bold);
        let requested_font = resolve_overlay_font_name(op, requested_bold);
        let original_color = graphics_state
            .fill_rgb
            .or_else(|| graphics_state.fill_gray.map(|gray| (gray, gray, gray)))
            .or(Some((0.0, 0.0, 0.0)));
        let requested_color = parse_rgb_color(op.color.as_deref()).or(original_color);
        let mut requested_x = state.x;
        if is_plate_operation(op) {
            if let Some(centered_x) = center_text_on_page_if_needed(page_width, op, safe_new_text, requested_bold, requested_size) {
                requested_x = centered_x;
            }
        }
        info!(
            "📏 Tamaño de fuente - Base: {:.3}, Escalado: {:.3}, Matriz: [{:.3}, {:.3}, {:.3}, {:.3}, {:.3}, {:.3}]",
            state.font_size,
            state.font_size_scaled,
            state.text_matrix[0],
            state.text_matrix[1],
            state.text_matrix[2],
            state.text_matrix[3],
            state.text_matrix[4],
            state.text_matrix[5]
        );
        overlays.push(DrawTextOp {
            x: requested_x,
            y: state.y,
            size: requested_size,
            is_bold: requested_bold,
            font_name: requested_font,
            color_rgb: requested_color,
            text: safe_new_text.to_string(),
        });
        return 1;
    }

    0
}

fn is_match_for_operation(state: &TextState, op: &OperationPayload, current_text: &str) -> bool {
    if let (Some(target_x), Some(target_y)) = (op.x, op.y) {
        let source_w = op.width.unwrap_or(48.0).abs();
        let source_h = op.height.unwrap_or((state.font_size * 1.2).max(12.0)).abs();
        let tolerance_x = (source_w / 2.0).clamp(6.0, 90.0);
        let tolerance_y = (source_h / 2.0).clamp(4.0, 40.0);

        let dx = (state.x - target_x).abs();
        let dy = (state.y - target_y).abs();

        if dx > tolerance_x || dy > tolerance_y {
            return false;
        }
    }

    if let Some(target_text) = op.target_text.as_ref().map(|text| text.trim()) {
        if !target_text.is_empty() {
            let normalized_target = normalize_match_text(target_text);
            let normalized_current = normalize_match_text(current_text);

            if !normalized_current.contains(&normalized_target) {
                let label_match = target_label_matches(&normalized_target, &normalized_current);
                warn!(
                    "Coordenada matcheada pero texto no coincide. target='{}' current='{}' normalizedTarget='{}' normalizedCurrent='{}' labelMatch={}",
                    target_text,
                    current_text,
                    normalized_target,
                    normalized_current,
                    label_match
                );
                if !label_match {
                    return false;
                }
            }
        }
    }

    true
}

fn normalize_match_text(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            'Á' | 'À' | 'Â' | 'Ã' | 'Ä' | 'á' | 'à' | 'â' | 'ã' | 'ä' => 'A',
            'É' | 'È' | 'Ê' | 'Ë' | 'é' | 'è' | 'ê' | 'ë' => 'E',
            'Í' | 'Ì' | 'Î' | 'Ï' | 'í' | 'ì' | 'î' | 'ï' => 'I',
            'Ó' | 'Ò' | 'Ô' | 'Õ' | 'Ö' | 'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'O',
            'Ú' | 'Ù' | 'Û' | 'Ü' | 'ú' | 'ù' | 'û' | 'ü' => 'U',
            'Ñ' | 'ñ' => 'N',
            'ç' | 'Ç' => 'C',
            '�' => 'I',
            _ if ch.is_ascii() => ch,
            _ => ' ',
        })
        .collect::<String>()
        .replace('\n', " ")
        .replace('\r', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_uppercase()
}

fn target_label_matches(normalized_target: &str, normalized_current: &str) -> bool {
    if let Some((label, _)) = normalized_target.split_once(':') {
        let clean = label.trim();
        if clean.len() >= 4 {
            if normalized_current.contains(clean) {
                return true;
            }

            let target_tokens: Vec<&str> = clean.split_whitespace().collect();
            let current_label = normalized_current
                .split_once(':')
                .map(|(left, _)| left)
                .unwrap_or(normalized_current)
                .trim();
            let current_tokens: Vec<&str> = current_label.split_whitespace().collect();

            if target_tokens.is_empty() || current_tokens.is_empty() {
                return false;
            }

            if target_tokens[0] != current_tokens[0] {
                return false;
            }

            if target_tokens.len() == 1 || current_tokens.len() == 1 {
                return true;
            }

            let target_second = target_tokens[1];
            let current_second = current_tokens[1];
            let prefix_len = target_second.len().min(current_second.len()).min(4);

            if prefix_len >= 3
                && target_second[..prefix_len] == current_second[..prefix_len]
            {
                return true;
            }
        }
    }

    false
}

fn resolve_overlay_font_name(op: &OperationPayload, bold: bool) -> Vec<u8> {
    let family = op
        .font_family
        .as_deref()
        .map(|value| value.trim().to_lowercase())
        .unwrap_or_else(|| "helvetica".to_string());

    match family.as_str() {
        "arial" => {
            if bold {
                b"FCOArialBold".to_vec()
            } else {
                b"FCOArialRegular".to_vec()
            }
        }
        "helvetica" => {
            if bold {
                b"FCOHelveticaBold".to_vec()
            } else {
                b"FCOHelveticaRegular".to_vec()
            }
        }
        _ => {
            if bold {
                b"FCOHelveticaBold".to_vec()
            } else {
                b"FCOHelveticaRegular".to_vec()
            }
        }
    }
}

fn is_plate_operation(op: &OperationPayload) -> bool {
    let is_plate_source = op
        .source_key
        .as_deref()
        .map(|value| value.eq_ignore_ascii_case("placa"))
        .unwrap_or(false);

    let is_plate_field = op
        .field_id
        .as_deref()
        .map(|value| {
            let normalized = value.trim().to_ascii_lowercase();
            normalized.contains("placa")
                || normalized.contains("plate")
                || normalized == "placa_arriba"
                || normalized == "placa_abajo"
        })
        .unwrap_or(false);

    is_plate_source || is_plate_field
}

fn center_text_on_page_if_needed(
    page_width: Option<f32>,
    op: &OperationPayload,
    text: &str,
    bold: bool,
    size: f32,
) -> Option<f32> {
    let text_len = text.trim().chars().count();
    if text_len == 7 {
        return None;
    }

    let page_w = page_width.unwrap_or(0.0);
    if page_w <= 0.0 {
        return op.x;
    }

    let family = op
        .font_family
        .as_deref()
        .map(|v| v.trim().to_lowercase())
        .unwrap_or_else(|| "helvetica".to_string());

    let width = measure_text_width_points(text, &family, bold, size.max(1.0)).unwrap_or(0.0);

    let centered_x = ((page_w - width) / 2.0).max(0.0);
    Some(centered_x)
}

fn get_page_width(document: &LoDocument, page_id: lopdf::ObjectId) -> Option<f32> {
    let page_obj = document.get_object(page_id).ok()?;
    let page_dict = page_obj.as_dict().ok()?;
    let media_box = page_dict.get(b"MediaBox").ok()?;

    let media_box_array = match media_box {
        Object::Array(arr) => arr,
        Object::Reference(id) => document.get_object(*id).ok()?.as_array().ok()?,
        _ => return None,
    };

    if media_box_array.len() != 4 {
        return None;
    }

    let llx = as_f32(&media_box_array[0])?;
    let urx = as_f32(&media_box_array[2])?;
    Some((urx - llx).abs())
}

fn measure_text_width_points(text: &str, family: &str, bold: bool, font_size: f32) -> Option<f32> {
    let font_file = match (family, bold) {
        ("arial", true) => "ARIALBD.TTF",
        ("arial", false) => "ARIAL.TTF",
        (_, true) => "Helvetica-Bold.ttf",
        (_, false) => "Helvetica.ttf",
    };

    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("fonts")
        .join(font_file);
    let bytes = std::fs::read(path).ok()?;
    let face = Face::parse(&bytes, 0).ok()?;
    let units_per_em = face.units_per_em() as f32;

    let mut total_units: f32 = 0.0;
    for ch in text.chars() {
        let advance = face
            .glyph_index(ch)
            .and_then(|gid| face.glyph_hor_advance(gid))
            .map(|v| v as f32)
            .unwrap_or(units_per_em * 0.5);
        total_units += advance;
    }

    Some((total_units / units_per_em) * font_size)
}

fn parse_rgb_color(color: Option<&str>) -> Option<(f32, f32, f32)> {
    let value = color?.trim();
    let hex = value.strip_prefix('#').unwrap_or(value);
    if hex.len() != 6 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }

    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;

    Some((r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0))
}

fn collect_bold_font_keys(document: &LoDocument, page_id: lopdf::ObjectId) -> Result<HashSet<Vec<u8>>, AppError> {
    let mut keys = HashSet::new();

    let page_obj = document
        .get_object(page_id)
        .map_err(|e| AppError::Pdfium(format!("No se pudo abrir pagina para analizar fuentes: {e}")))?;
    let page_dict = page_obj
        .as_dict()
        .map_err(|e| AppError::Pdfium(format!("Pagina no es diccionario: {e}")))?;

    let resources_obj = match page_dict.get(b"Resources") {
        Ok(obj) => obj,
        Err(_) => return Ok(keys),
    };

    let resources_dict = match resources_obj {
        Object::Dictionary(dict) => dict,
        Object::Reference(id) => document
            .get_object(*id)
            .map_err(|e| AppError::Pdfium(format!("No se pudo abrir Resources ref: {e}")))?
            .as_dict()
            .map_err(|e| AppError::Pdfium(format!("Resources ref no es diccionario: {e}")))?,
        _ => return Ok(keys),
    };

    let fonts_obj = match resources_dict.get(b"Font") {
        Ok(obj) => obj,
        Err(_) => return Ok(keys),
    };

    let fonts_dict = match fonts_obj {
        Object::Dictionary(dict) => dict,
        Object::Reference(id) => document
            .get_object(*id)
            .map_err(|e| AppError::Pdfium(format!("No se pudo abrir Font resources ref: {e}")))?
            .as_dict()
            .map_err(|e| AppError::Pdfium(format!("Font resources ref no es diccionario: {e}")))?,
        _ => return Ok(keys),
    };

    for (key, font_obj) in fonts_dict.iter() {
        let font_dict = match font_obj {
            Object::Dictionary(dict) => dict,
            Object::Reference(id) => {
                let Some(obj) = document.get_object(*id).ok() else {
                    continue;
                };
                let Some(dict) = obj.as_dict().ok() else {
                    continue;
                };
                dict
            }
            _ => continue,
        };

        if let Ok(Object::Name(base_font)) = font_dict.get(b"BaseFont") {
            let name = String::from_utf8_lossy(base_font).to_lowercase();
            if name.contains("bold") {
                keys.insert(key.clone());
            }
        }
    }

    Ok(keys)
}

fn decode_pdf_string(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        let utf16_units: Vec<u16> = bytes[2..]
            .chunks(2)
            .filter_map(|chunk| {
                if chunk.len() == 2 {
                    Some(u16::from_be_bytes([chunk[0], chunk[1]]))
                } else {
                    None
                }
            })
            .collect();

        return String::from_utf16_lossy(&utf16_units);
    }

    String::from_utf8_lossy(bytes).to_string()
}

fn as_f32(object: &Object) -> Option<f32> {
    match object {
        Object::Integer(value) => Some(*value as f32),
        Object::Real(value) => Some(*value),
        _ => None,
    }
}

