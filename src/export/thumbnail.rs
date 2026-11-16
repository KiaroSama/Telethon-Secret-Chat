//! Image file thumbnails, including the Desktop JPEG and optional baseline paths.
use super::{
    files::{prepare_relative_path, safe_relative_path},
    qt_jpeg,
};
use image::{DynamicImage, GenericImageView, ImageFormat};
use std::{fs, io::Cursor, path::Path};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThumbnailEncoder {
    #[default]
    Desktop,
    Baseline,
}
#[derive(Debug, Clone)]
pub struct ThumbnailOptions<'a> {
    pub image_format: Option<&'a str>,
    pub quality: Option<u8>,
    pub postfix: &'a str,
    pub jpeg_encoder: ThumbnailEncoder,
}
impl Default for ThumbnailOptions<'_> {
    fn default() -> Self {
        Self {
            image_format: None,
            quality: None,
            postfix: "_thumb",
            jpeg_encoder: ThumbnailEncoder::Desktop,
        }
    }
}
/// Returns an empty path and zero dimensions on unreadable, oversized, invalid,
/// or unwritable images, matching Desktop's WriteImageThumb failure contract.
pub fn write_image_thumb<F>(
    base: &Path,
    large_path: &str,
    convert_size: F,
    options: &ThumbnailOptions<'_>,
) -> (String, (u32, u32))
where
    F: FnOnce((u32, u32)) -> (u32, u32),
{
    let empty = (String::new(), (0, 0));
    if !safe_relative_path(large_path) {
        return empty;
    }
    let Ok(source) = fs::read(base.join(large_path)) else {
        return empty;
    };
    let Ok(source_format) = image::guess_format(&source) else {
        return empty;
    };
    let Ok(reader) = image::ImageReader::new(Cursor::new(&source)).with_guessed_format() else {
        return empty;
    };
    let Ok((width, height)) = reader.into_dimensions() else {
        return empty;
    };
    if width == 0 || height == 0 || width >= 10000 || height >= 10000 {
        return empty;
    }
    let final_size = convert_size((width, height));
    if final_size.0 == 0 || final_size.1 == 0 || final_size.0 >= 10000 || final_size.1 >= 10000 {
        return empty;
    }
    let file_start = large_path.rfind('/').map_or(0, |i| i + 1);
    let first_dot = large_path[file_start..].find('.').map(|i| file_start + i);
    let thumb = first_dot.map_or_else(
        || format!("{large_path}{}", options.postfix),
        |i| {
            format!(
                "{}{}{}",
                &large_path[..i],
                options.postfix,
                &large_path[i..]
            )
        },
    );
    if !safe_relative_path(&thumb) {
        return empty;
    }
    let result = prepare_relative_path(base, &thumb);
    let save_format = match options.image_format {
        None => source_format,
        Some(name) => match name.to_ascii_uppercase().as_str() {
            "JPEG" | "JPG" | "MPO" => ImageFormat::Jpeg,
            "PNG" => ImageFormat::Png,
            "GIF" => ImageFormat::Gif,
            "WEBP" => ImageFormat::WebP,
            "BMP" => ImageFormat::Bmp,
            "TIFF" | "TIF" => ImageFormat::Tiff,
            _ => return empty,
        },
    };
    let bytes = if source_format == ImageFormat::Jpeg
        && save_format == ImageFormat::Jpeg
        && options.jpeg_encoder == ThumbnailEncoder::Desktop
    {
        match qt_jpeg::desktop_thumb(&source, final_size, options.quality) {
            Ok(bytes) => bytes,
            Err(_) => return empty,
        }
    } else {
        let Ok(mut image) = image::load_from_memory_with_format(&source, source_format) else {
            return empty;
        };
        if source_format == ImageFormat::Jpeg {
            image = DynamicImage::ImageRgb8(image.into_rgb8());
        }
        if image.dimensions() != final_size {
            image = image.resize_exact(
                final_size.0,
                final_size.1,
                image::imageops::FilterType::Triangle,
            );
        }
        let mut cursor = Cursor::new(Vec::new());
        let encoded = if save_format == ImageFormat::Jpeg {
            let rgb = image.into_rgb8();
            let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
                &mut cursor,
                options.quality.unwrap_or(75).min(100),
            );
            encoder.encode_image(&rgb)
        } else {
            image.write_to(&mut cursor, save_format)
        };
        if encoded.is_err() {
            return empty;
        }
        cursor.into_inner()
    };
    if fs::write(base.join(&result), bytes).is_err() {
        return empty;
    }
    (result, final_size)
}
