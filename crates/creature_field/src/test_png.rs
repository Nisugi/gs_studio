//! Test-only PNG encode/decode for this crate's tests: the `png` codec as a
//! dev-dependency (this crate must not depend on `image`). A copy of
//! `gs_base::png_meta`'s own, which a test here cannot reach.

/// A valid 1x1 opaque-white RGBA PNG.
pub fn tiny_png_bytes() -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[0xff; 4]).unwrap();
    }
    out
}

/// Whether the bytes still decode as a PNG, pixels included.
pub fn decodes(bytes: &[u8]) -> bool {
    let Ok(mut reader) = png::Decoder::new(std::io::Cursor::new(bytes)).read_info() else {
        return false;
    };
    let mut buf = vec![0; reader.output_buffer_size()];
    reader.next_frame(&mut buf).is_ok()
}
