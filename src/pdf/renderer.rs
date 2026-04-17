use lopdf::content::{Content, Operation};
use lopdf::{Object, StringFormat};

use crate::pdf::font::OverlayFontNames;
use crate::pdf::utils::encode_utf16_be_no_bom;

#[derive(Debug, Clone)]
pub struct DrawTextOp {
    pub x: f32,
    pub y: f32,
    pub size: f32,
    pub is_bold: bool,
    pub font_name: Vec<u8>,
    pub color_rgb: Option<(f32, f32, f32)>,
    pub text: String,
}

pub fn append_unicode_text_operations(content: &mut Content, fonts: &OverlayFontNames, draws: &[DrawTextOp]) {
    for draw in draws {
        let encoded_text = encode_utf16_be_no_bom(&draw.text);
        let default_name = if draw.is_bold { &fonts.bold } else { &fonts.regular };
        let font_name = if draw.font_name.is_empty() {
            default_name.to_vec()
        } else {
            draw.font_name.clone()
        };

        if let Some((r, g, b)) = draw.color_rgb {
            content.operations.push(Operation::new(
                "rg",
                vec![Object::Real(r), Object::Real(g), Object::Real(b)],
            ));
        }

        content.operations.push(Operation::new("BT", vec![]));
        content.operations.push(Operation::new(
            "Tf",
            vec![
                Object::Name(font_name),
                Object::Real(draw.size.max(1.0)),
            ],
        ));
        content.operations.push(Operation::new(
            "Tm",
            vec![
                Object::Integer(1),
                Object::Integer(0),
                Object::Integer(0),
                Object::Integer(1),
                Object::Real(draw.x),
                Object::Real(draw.y),
            ],
        ));
        content.operations.push(Operation::new(
            "Tj",
            vec![Object::String(encoded_text, StringFormat::Hexadecimal)],
        ));
        content.operations.push(Operation::new("ET", vec![]));
    }
}
