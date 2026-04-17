use std::collections::BTreeSet;
use std::path::PathBuf;

use lopdf::{Dictionary, Document as LoDocument, Object};
use ttf_parser::Face;

use crate::error::AppError;

#[derive(Debug, Clone)]
pub struct OverlayFontNames {
    pub regular: Vec<u8>,
    pub bold: Vec<u8>,
}

pub fn ensure_unicode_overlay_fonts(
    document: &mut LoDocument,
    page_id: lopdf::ObjectId,
    used_utf16: &BTreeSet<u16>,
) -> Result<OverlayFontNames, AppError> {
    let regular_name = b"FCOHelveticaRegular".to_vec();
    let bold_name = b"FCOHelveticaBold".to_vec();
    let arial_regular_name = b"FCOArialRegular".to_vec();
    let arial_bold_name = b"FCOArialBold".to_vec();

    let fonts_id = ensure_page_font_resources(document, page_id)?;

    let regular_ttf = read_font_with_fallback("Helvetica.ttf")?;
    let bold_ttf = read_font_with_fallback("Helvetica-Bold.ttf").or_else(|_| read_font_with_fallback("Helvetica.ttf"))?;
    let arial_regular_ttf = read_font_with_fallback("ARIAL.TTF")?;
    let arial_bold_ttf = read_font_with_fallback("ARIALBD.TTF").or_else(|_| read_font_with_fallback("ARIAL.TTF"))?;

    let regular_font_id = embed_type0_unicode_ttf(document, "Helvetica", regular_ttf, used_utf16)?;
    let bold_font_id = embed_type0_unicode_ttf(document, "Helvetica-Bold", bold_ttf, used_utf16)?;
    let arial_regular_font_id = embed_type0_unicode_ttf(document, "ArialMT", arial_regular_ttf, used_utf16)?;
    let arial_bold_font_id = embed_type0_unicode_ttf(document, "Arial-BoldMT", arial_bold_ttf, used_utf16)?;

    let fonts_obj = document
        .get_object_mut(fonts_id)
        .map_err(|e| AppError::Pdfium(format!("No se pudo abrir Font mutable: {e}")))?;
    let fonts_dict = fonts_obj
        .as_dict_mut()
        .map_err(|e| AppError::Pdfium(format!("Font no es diccionario mutable: {e}")))?;

    fonts_dict.set(regular_name.clone(), Object::Reference(regular_font_id));
    fonts_dict.set(bold_name.clone(), Object::Reference(bold_font_id));
    fonts_dict.set(arial_regular_name, Object::Reference(arial_regular_font_id));
    fonts_dict.set(arial_bold_name, Object::Reference(arial_bold_font_id));

    Ok(OverlayFontNames {
        regular: regular_name,
        bold: bold_name,
    })
}

fn read_font_with_fallback(file_name: &str) -> Result<Vec<u8>, AppError> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src").join("fonts").join(file_name);
    std::fs::read(&path)
        .map_err(|e| AppError::Pdfium(format!("No se pudo leer fuente {}: {e}", path.display())))
}

fn embed_type0_unicode_ttf(
    document: &mut LoDocument,
    font_name: &str,
    font_data: Vec<u8>,
    used_utf16: &BTreeSet<u16>,
) -> Result<lopdf::ObjectId, AppError> {
    let cid_to_gid_map = create_cid_to_gid_map(&font_data, used_utf16)?;
    let cid_widths = create_cid_widths(&font_data, used_utf16)?;

    let font_file_stream = lopdf::Stream::new(
        Dictionary::from_iter(vec![(
            "Length1",
            Object::Integer(font_data.len() as i64),
        )]),
        font_data,
    );
    let font_file_id = document.add_object(font_file_stream);

    let mut font_descriptor = Dictionary::new();
    font_descriptor.set("Type", Object::Name(b"FontDescriptor".to_vec()));
    font_descriptor.set("FontName", Object::Name(font_name.as_bytes().to_vec()));
    font_descriptor.set("Flags", Object::Integer(32));
    font_descriptor.set(
        "FontBBox",
        Object::Array(vec![0.into(), (-200).into(), 1000.into(), 900.into()]),
    );
    font_descriptor.set("ItalicAngle", Object::Integer(0));
    font_descriptor.set("Ascent", Object::Integer(800));
    font_descriptor.set("Descent", Object::Integer(-200));
    font_descriptor.set("CapHeight", Object::Integer(700));
    font_descriptor.set("StemV", Object::Integer(80));
    font_descriptor.set("FontFile2", Object::Reference(font_file_id));
    let font_descriptor_id = document.add_object(Object::Dictionary(font_descriptor));

    let mut cid_font = Dictionary::new();
    cid_font.set("Type", Object::Name(b"Font".to_vec()));
    cid_font.set("Subtype", Object::Name(b"CIDFontType2".to_vec()));
    cid_font.set("BaseFont", Object::Name(font_name.as_bytes().to_vec()));
    let mut cid_system_info = Dictionary::new();
    cid_system_info.set("Registry", Object::String(b"Adobe".to_vec(), lopdf::StringFormat::Literal));
    cid_system_info.set("Ordering", Object::String(b"Identity".to_vec(), lopdf::StringFormat::Literal));
    cid_system_info.set("Supplement", Object::Integer(0));
    cid_font.set("CIDSystemInfo", Object::Dictionary(cid_system_info));
    cid_font.set("FontDescriptor", Object::Reference(font_descriptor_id));
    let cid_to_gid_id = document.add_object(lopdf::Stream::new(Dictionary::new(), cid_to_gid_map));
    cid_font.set("CIDToGIDMap", Object::Reference(cid_to_gid_id));
    cid_font.set("DW", Object::Integer(500));
    cid_font.set("W", cid_widths);
    let cid_font_id = document.add_object(Object::Dictionary(cid_font));

    let to_unicode_cmap = create_to_unicode_cmap(used_utf16);
    let to_unicode_id = document.add_object(lopdf::Stream::new(Dictionary::new(), to_unicode_cmap));

    let mut type0_font = Dictionary::new();
    type0_font.set("Type", Object::Name(b"Font".to_vec()));
    type0_font.set("Subtype", Object::Name(b"Type0".to_vec()));
    type0_font.set("BaseFont", Object::Name(font_name.as_bytes().to_vec()));
    type0_font.set("Encoding", Object::Name(b"Identity-H".to_vec()));
    type0_font.set("DescendantFonts", Object::Array(vec![Object::Reference(cid_font_id)]));
    type0_font.set("ToUnicode", Object::Reference(to_unicode_id));

    Ok(document.add_object(Object::Dictionary(type0_font)))
}

fn create_to_unicode_cmap(used_utf16: &BTreeSet<u16>) -> Vec<u8> {
    let mut codes: Vec<u16> = used_utf16.iter().copied().filter(|code| *code != 0).collect();
    if !codes.contains(&0x0020) {
        codes.push(0x0020);
    }
    codes.sort_unstable();
    codes.dedup();

    let mut cmap = String::new();
    cmap.push_str("/CIDInit /ProcSet findresource begin\n");
    cmap.push_str("12 dict begin\n");
    cmap.push_str("begincmap\n");
    cmap.push_str("/CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> def\n");
    cmap.push_str("/CMapName /FCOIdentityUnicode def\n");
    cmap.push_str("/CMapType 2 def\n");
    cmap.push_str("1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n");

    for chunk in codes.chunks(100) {
        cmap.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for code in chunk {
            cmap.push_str(&format!("<{0:04X}> <{0:04X}>\n", code));
        }
        cmap.push_str("endbfchar\n");
    }

    cmap.push_str("endcmap\n");
    cmap.push_str("CMapName currentdict /CMap defineresource pop\n");
    cmap.push_str("end\n");
    cmap.push_str("end\n");

    cmap.into_bytes()
}

fn create_cid_to_gid_map(font_data: &[u8], used_utf16: &BTreeSet<u16>) -> Result<Vec<u8>, AppError> {
    let face = Face::parse(font_data, 0)
        .map_err(|e| AppError::Pdfium(format!("No se pudo parsear TTF para CIDToGIDMap: {e:?}")))?;

    let mut max_cid = used_utf16.iter().copied().max().unwrap_or(0x0020);
    if max_cid < 0x0020 {
        max_cid = 0x0020;
    }

    let mut map = vec![0u8; (max_cid as usize + 1) * 2];

    for cid in used_utf16.iter().copied() {
        if let Some(ch) = char::from_u32(cid as u32) {
            if let Some(glyph_id) = face.glyph_index(ch) {
                let idx = cid as usize * 2;
                let gid = glyph_id.0;
                map[idx] = (gid >> 8) as u8;
                map[idx + 1] = (gid & 0xFF) as u8;
            }
        }
    }

    Ok(map)
}

fn create_cid_widths(font_data: &[u8], used_utf16: &BTreeSet<u16>) -> Result<Object, AppError> {
    let face = Face::parse(font_data, 0)
        .map_err(|e| AppError::Pdfium(format!("No se pudo parsear TTF para anchos CID: {e:?}")))?;

    let units_per_em = face.units_per_em() as f32;
    let mut width_entries: Vec<Object> = Vec::new();

    for cid in used_utf16.iter().copied() {
        if let Some(ch) = char::from_u32(cid as u32) {
            if let Some(glyph_id) = face.glyph_index(ch) {
                let advance = face.glyph_hor_advance(glyph_id).unwrap_or(500) as f32;
                let width_1000 = ((advance * 1000.0) / units_per_em).round() as i64;
                width_entries.push(Object::Integer(cid as i64));
                width_entries.push(Object::Array(vec![Object::Integer(width_1000.max(100))]));
            }
        }
    }

    Ok(Object::Array(width_entries))
}

fn ensure_page_font_resources(
    document: &mut LoDocument,
    page_id: lopdf::ObjectId,
) -> Result<lopdf::ObjectId, AppError> {
    let existing_resources_obj = {
        let page_obj = document
            .get_object(page_id)
            .map_err(|e| AppError::Pdfium(format!("No se pudo abrir pagina: {e}")))?;
        let page_dict = page_obj
            .as_dict()
            .map_err(|e| AppError::Pdfium(format!("Pagina no es diccionario: {e}")))?;
        page_dict.get(b"Resources").ok().cloned()
    };

    let resources_id = match existing_resources_obj {
        Some(Object::Reference(id)) => id,
        Some(Object::Dictionary(dict)) => {
            let id = document.add_object(Object::Dictionary(dict));
            let page_obj = document
                .get_object_mut(page_id)
                .map_err(|e| AppError::Pdfium(format!("No se pudo abrir pagina mutable: {e}")))?;
            let page_dict = page_obj
                .as_dict_mut()
                .map_err(|e| AppError::Pdfium(format!("Pagina mutable no es diccionario: {e}")))?;
            page_dict.set("Resources", Object::Reference(id));
            id
        }
        _ => {
            let id = document.add_object(Object::Dictionary(Dictionary::new()));
            let page_obj = document
                .get_object_mut(page_id)
                .map_err(|e| AppError::Pdfium(format!("No se pudo abrir pagina mutable: {e}")))?;
            let page_dict = page_obj
                .as_dict_mut()
                .map_err(|e| AppError::Pdfium(format!("Pagina mutable no es diccionario: {e}")))?;
            page_dict.set("Resources", Object::Reference(id));
            id
        }
    };

    let existing_fonts_obj = {
        let resources_obj = document
            .get_object(resources_id)
            .map_err(|e| AppError::Pdfium(format!("No se pudo abrir Resources: {e}")))?;
        let resources_dict = resources_obj
            .as_dict()
            .map_err(|e| AppError::Pdfium(format!("Resources no es diccionario: {e}")))?;
        resources_dict.get(b"Font").ok().cloned()
    };

    let fonts_id = match existing_fonts_obj {
        Some(Object::Reference(id)) => id,
        Some(Object::Dictionary(dict)) => {
            let id = document.add_object(Object::Dictionary(dict));
            let resources_obj = document
                .get_object_mut(resources_id)
                .map_err(|e| AppError::Pdfium(format!("No se pudo abrir Resources mutable: {e}")))?;
            let resources_dict = resources_obj
                .as_dict_mut()
                .map_err(|e| AppError::Pdfium(format!("Resources mutable no es diccionario: {e}")))?;
            resources_dict.set("Font", Object::Reference(id));
            id
        }
        _ => {
            let id = document.add_object(Object::Dictionary(Dictionary::new()));
            let resources_obj = document
                .get_object_mut(resources_id)
                .map_err(|e| AppError::Pdfium(format!("No se pudo abrir Resources mutable: {e}")))?;
            let resources_dict = resources_obj
                .as_dict_mut()
                .map_err(|e| AppError::Pdfium(format!("Resources mutable no es diccionario: {e}")))?;
            resources_dict.set("Font", Object::Reference(id));
            id
        }
    };

    Ok(fonts_id)
}
