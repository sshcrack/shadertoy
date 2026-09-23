use super::*;

pub(super) struct BufferOverride {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub(super) fn load_overrides(
    assignments: &[String],
    manifest: &crate::manifest::Manifest,
    output_width: u32,
    output_height: u32,
) -> Result<Vec<BufferOverride>> {
    let mut result = Vec::new();
    for assignment in assignments {
        let (name, path) = split_assignment(assignment)?;
        let pass = manifest
            .passes
            .iter()
            .find(|pass| pass.name == name)
            .with_context(|| format!("unknown buffer override pass '{name}'"))?;
        if !matches!(
            pass.kind,
            PassKind::Buffer | PassKind::Compute | PassKind::Mesh
        ) {
            bail!(
                "buffer override '{}' is not a 2D buffer/compute/mesh pass",
                name
            );
        }
        let (width, height) = manifest.pass_dimensions(pass, output_width, output_height);
        let image = ImageReader::open(path)
            .with_context(|| format!("failed to open buffer override {}", path.display()))?
            .decode()
            .with_context(|| format!("failed to decode buffer override {}", path.display()))?
            .to_rgba8();
        let dimensions = image.dimensions();
        if dimensions != (width, height) {
            bail!(
                "buffer override '{}' is {}x{} but pass '{}' is {}x{}; use an exact-size image",
                name,
                dimensions.0,
                dimensions.1,
                name,
                width,
                height
            );
        }
        let mut rgba = image.into_raw();
        flip_rgba_rows(&mut rgba, width, height);
        result.push(BufferOverride {
            name: name.to_string(),
            width,
            height,
            rgba,
        });
    }
    Ok(result)
}

pub(super) fn split_assignment(value: &str) -> Result<(&str, &Path)> {
    let (name, path) = value
        .split_once('=')
        .with_context(|| format!("expected BUFFER=IMAGE, got '{value}'"))?;
    if name.trim().is_empty() || path.trim().is_empty() {
        bail!("expected non-empty BUFFER=IMAGE assignment, got '{value}'");
    }
    Ok((name.trim(), Path::new(path.trim())))
}

pub(super) fn save_rgb_png(image: &RgbImage, output: &Path) -> Result<()> {
    let mut pixels = image.pixels.clone();
    flip_rgb_rows(&mut pixels, image.width, image.height);
    ::image::save_buffer(
        output,
        &pixels,
        image.width,
        image.height,
        ::image::ColorType::Rgb8,
    )
    .with_context(|| format!("failed to write PNG {}", output.display()))
}

pub fn preview_raw_rgb_bytes(image: &RgbImage) -> Result<Vec<u8>> {
    let payload_len = image
        .pixels
        .len()
        .checked_add(8)
        .context("preview raw frame size overflow")?;
    let mut payload = Vec::with_capacity(payload_len);
    payload.extend_from_slice(&image.width.to_le_bytes());
    payload.extend_from_slice(&image.height.to_le_bytes());
    payload.extend_from_slice(&image.pixels);
    Ok(payload)
}

pub fn preview_png_bytes(image: &RgbImage) -> Result<Vec<u8>> {
    use ::image::ImageEncoder;
    use ::image::codecs::png::{CompressionType, FilterType, PngEncoder};

    let mut pixels = image.pixels.clone();
    flip_rgb_rows(&mut pixels, image.width, image.height);
    let mut encoded = Vec::new();
    PngEncoder::new_with_quality(
        &mut encoded,
        CompressionType::Level(1),
        FilterType::Adaptive,
    )
    .write_image(
        &pixels,
        image.width,
        image.height,
        ::image::ExtendedColorType::Rgb8,
    )
    .context("failed to encode preview PNG")?;
    Ok(encoded)
}

pub fn preview_jpeg_bytes(image: &RgbImage) -> Result<Vec<u8>> {
    use ::image::codecs::jpeg::JpegEncoder;

    let mut pixels = image.pixels.clone();
    flip_rgb_rows(&mut pixels, image.width, image.height);
    let mut encoded = Vec::new();
    JpegEncoder::new_with_quality(&mut encoded, 90)
        .encode(
            &pixels,
            image.width,
            image.height,
            ::image::ExtendedColorType::Rgb8,
        )
        .context("failed to encode preview JPEG")?;
    Ok(encoded)
}

pub fn flip_rgb_rows(data: &mut [u8], width: u32, height: u32) {
    flip_rows(data, width, height, 3);
}

pub(super) fn flip_rgba_rows(data: &mut [u8], width: u32, height: u32) {
    flip_rows(data, width, height, 4);
}

fn flip_rows(data: &mut [u8], width: u32, height: u32, channels: usize) {
    let Some(row) = (width as usize).checked_mul(channels) else {
        debug_assert!(false, "row size overflow");
        return;
    };
    let Some(expected_len) = row.checked_mul(height as usize) else {
        debug_assert!(false, "image size overflow");
        return;
    };
    debug_assert_eq!(data.len(), expected_len);
    for y in 0..(height as usize / 2) {
        let opposite = height as usize - 1 - y;
        let (before_opposite, opposite_and_after) = data.split_at_mut(opposite * row);
        let top = &mut before_opposite[y * row..(y + 1) * row];
        let bottom = &mut opposite_and_after[..row];
        top.swap_with_slice(bottom);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_image() -> RgbImage {
        RgbImage::new(2, 2, vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255])
    }

    #[test]
    fn preview_jpeg_is_valid() {
        let jpeg = preview_jpeg_bytes(&test_image()).unwrap();
        let decoded = ::image::load_from_memory_with_format(&jpeg, ::image::ImageFormat::Jpeg)
            .unwrap()
            .to_rgb8();
        assert_eq!(decoded.dimensions(), (2, 2));
    }

    #[test]
    fn preview_png_is_valid_and_flips_gl_rows() {
        let png = preview_png_bytes(&test_image()).unwrap();
        let decoded = ::image::load_from_memory_with_format(&png, ::image::ImageFormat::Png)
            .unwrap()
            .to_rgb8();
        assert_eq!(
            decoded.into_raw(),
            vec![0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255, 0,]
        );
    }

    #[test]
    fn preview_raw_rgb_preserves_gl_pixels_and_dimensions() {
        let image = test_image();
        let raw = preview_raw_rgb_bytes(&image).unwrap();
        assert_eq!(&raw[..4], &2u32.to_le_bytes());
        assert_eq!(&raw[4..8], &2u32.to_le_bytes());
        assert_eq!(&raw[8..], image.pixels.as_slice());
    }
}
