//! Theme image loader & resizer.
//! Port of `src/theme/image_loader.{cpp,h}`, ICO decoder tests, data URI loader tests,
//! and image source logging tests.

use base64::Engine;
use image::DynamicImage;
use image::imageops::FilterType;
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::scheme::Scheme;

/// A 112x112 RGB pixel buffer ready for quantization/clustering. No alpha.
/// 112 * 112 * 3 = 37,632 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadedImage {
    pub rgb: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

impl Default for LoadedImage {
    fn default() -> Self {
        Self {
            rgb: vec![0; 112 * 112 * 3],
            width: 112,
            height: 112,
        }
    }
}

pub fn load_and_resize(path: &Path, scheme: Scheme) -> Result<LoadedImage, String> {
    let img =
        image::open(path).map_err(|e| format!("failed to load image {}: {e}", path.display()))?;
    resize_loaded_image(&img, scheme)
}

pub fn load_from_memory_and_resize(bytes: &[u8], scheme: Scheme) -> Result<LoadedImage, String> {
    let img = image::load_from_memory(bytes).map_err(|e| format!("failed to decode image: {e}"))?;
    resize_loaded_image(&img, scheme)
}

pub fn resize_loaded_image(img: &DynamicImage, scheme: Scheme) -> Result<LoadedImage, String> {
    let filter = if scheme.is_material() {
        FilterType::Triangle
    } else {
        FilterType::CatmullRom
    };

    let resized = img.resize_exact(112, 112, filter);
    let rgb_img = resized.to_rgb8();

    Ok(LoadedImage {
        rgb: rgb_img.into_raw(),
        width: 112,
        height: 112,
    })
}

pub fn decode_data_uri(uri: &str) -> Result<DynamicImage, String> {
    if !uri.starts_with("data:") {
        return Err("not a data URI".to_string());
    }

    let comma_idx = uri
        .find(',')
        .ok_or_else(|| "missing comma separator in data URI".to_string())?;
    let header = &uri[..comma_idx];
    let payload = &uri[comma_idx + 1..];

    if !header.contains(";base64") {
        return Err("data URI is not base64 encoded".to_string());
    }

    let decoded_bytes = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .map_err(|e| format!("failed to decode base64 payload: {e}"))?;

    image::load_from_memory(&decoded_bytes)
        .map_err(|e| format!("failed to decode image bytes: {e}"))
}

pub fn describe_image_source(source: &str) -> String {
    if source.starts_with("data:") {
        if let Some(comma_idx) = source.find(',') {
            let header = &source[..comma_idx];
            let payload = &source[comma_idx + 1..];
            format!(
                "{} (payload={} bytes, uri={} bytes)",
                header,
                payload.len(),
                source.len()
            )
        } else {
            let header_len = source.len().min(160);
            format!(
                "malformed data URI (header={}, original={} bytes)",
                &source[..header_len],
                source.len()
            )
        }
    } else if source.len() > 256 {
        format!("{}... (original={} bytes)", &source[..256], source.len())
    } else {
        source.to_string()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn data_uri_decodes_mismatched_media_types() {
        let mut png_bytes = Vec::new();
        let dyn_img = DynamicImage::ImageRgb8(image::RgbImage::new(1, 1));
        dyn_img
            .write_to(
                &mut std::io::Cursor::new(&mut png_bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        let b64 = base64::engine::general_purpose::STANDARD.encode(&png_bytes);
        let png_declared_jpeg = format!("data:image/jpeg;base64,{b64}");

        let img = decode_data_uri(&png_declared_jpeg).expect("data URI decode should succeed");
        assert_eq!(img.width(), 1);
        assert_eq!(img.height(), 1);
    }

    #[test]
    fn data_uri_invalid_base64_fails() {
        let err = decode_data_uri("data:image/png;base64,not_base64!").unwrap_err();
        assert!(err.contains("base64"));
    }

    #[test]
    fn data_uri_missing_comma_fails() {
        let err = decode_data_uri("data:image/png;base64").unwrap_err();
        assert!(err.contains("separator"));
    }

    #[test]
    fn describe_image_source_formats_correctly() {
        assert_eq!(describe_image_source("/tmp/icon.png"), "/tmp/icon.png");

        let data_uri = "data:image/png;base64,abcdef";
        let desc = describe_image_source(data_uri);
        assert_eq!(
            desc,
            "data:image/png;base64 (payload=6 bytes, uri=28 bytes)"
        );
        assert!(!desc.contains("abcdef"));

        let long_path = "x".repeat(800);
        let long_desc = describe_image_source(&long_path);
        assert!(long_desc.contains("original=800 bytes"));

        let malformed = format!("data:{}", "h".repeat(160));
        let malformed_desc = describe_image_source(&malformed);
        assert!(malformed_desc.contains("malformed"));
    }

    #[test]
    fn resize_image_returns_112_by_112_rgb() {
        let img = DynamicImage::ImageRgb8(image::RgbImage::new(500, 300));
        let loaded = resize_loaded_image(&img, Scheme::TonalSpot).unwrap();
        assert_eq!(loaded.width, 112);
        assert_eq!(loaded.height, 112);
        assert_eq!(loaded.rgb.len(), 112 * 112 * 3);
    }
}
