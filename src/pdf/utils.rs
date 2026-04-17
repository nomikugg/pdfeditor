pub fn encode_utf16_be_no_bom(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() * 2);

    for unit in text.encode_utf16() {
        out.extend_from_slice(&unit.to_be_bytes());
    }

    out
}
