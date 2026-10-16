//! Images (C-009): which extensions decode, how a raster file becomes frames of RGBA
//! pixels with its metadata, and how it is written back in another raster format. SVG
//! and fonts need the window's renderers and are handled by the view; this module only
//! names them and unpacks svgz.

use std::io::{Cursor, Read};

use image::codecs::gif::GifDecoder;
use image::codecs::png::PngDecoder;
use image::codecs::webp::WebPDecoder;
use image::{AnimationDecoder, ColorType, DynamicImage, ImageDecoder, ImageFormat, ImageReader, RgbaImage};

/// What an image file is, by its extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Pixels, decoded by the image crate.
    Raster,
    /// SVG, plain or gzip-compressed, rendered as vectors by the window.
    Vector,
    /// A TrueType font, shown as a specimen.
    Font,
}

/// Extensions the viewer decodes, lower-case, as the clause lists them.
pub const RASTER: &[&str] = &[
    "png", "apng", "jpg", "jpeg", "jfif", "gif", "bmp", "dib", "ico", "cur", "tif", "tiff", "webp", "tga",
    "targa", "pbm", "pgm", "ppm", "pnm", "pam", "dds", "exr", "hdr", "qoi", "ff",
];
pub const VECTOR: &[&str] = &["svg", "svgz"];
pub const FONT: &[&str] = &["ttf"];

/// Extensions of the family that have no decoder here yet; their tabs say so.
pub const UNSUPPORTED: &[&str] = &[
    "aai", "avif", "cin", "dcm", "dicom", "dfont", "dpx", "emf", "emz", "heic", "heif", "jls", "jng", "jp2",
    "jpc", "jpx", "jxl", "jxr", "ora", "pcd", "pes", "psb", "psd", "sfw", "wmf", "wmz", "bw", "cdg", "miff",
    "mvg", "pcx", "pix", "ptx", "rgb", "rgba", "sgi", "txd", "wbmp", "xbm", "xface", "xpm",
];

/// Raster formats a save-as writes without loss for 8-bit RGBA pixels.
pub const LOSSLESS: &[&str] = &["png", "bmp", "tif", "tiff", "tga", "qoi", "ff", "webp", "pam"];

/// Raster formats a save-as can write at all.
pub const WRITABLE: &[&str] = &["png", "jpg", "jpeg", "gif", "bmp", "ico", "tif", "tiff", "webp", "tga", "pnm", "pam", "qoi", "ff", "exr", "hdr"];

/// The kind of image a file name denotes, by its extension; `None` for an extension
/// the viewer has no decoder for.
pub fn kind_of(name: &str) -> Option<Kind> {
    let ext = name.rsplit('.').next()?.to_ascii_lowercase();
    if RASTER.contains(&ext.as_str()) {
        Some(Kind::Raster)
    } else if VECTOR.contains(&ext.as_str()) {
        Some(Kind::Vector)
    } else if FONT.contains(&ext.as_str()) {
        Some(Kind::Font)
    } else {
        None
    }
}

/// One frame of pixels, RGBA, 8 bits per channel, row-major.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub rgba: Vec<u8>,
    /// How long the frame shows in an animation; 0 for a still image.
    pub delay_ms: u32,
}

/// A decoded raster image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    /// The pixel format of the file, such as "RGBA, 8 bits per channel".
    pub colour: String,
    pub frames: Vec<Frame>,
    /// EXIF fields worth showing, in a fixed order, as `(name, value)`.
    pub exif: Vec<(String, String)>,
}

impl Picture {
    pub fn is_animated(&self) -> bool {
        self.frames.len() > 1
    }

    /// The first frame as an image-crate buffer.
    pub fn first(&self) -> RgbaImage {
        RgbaImage::from_raw(self.width, self.height, self.frames[0].rgba.clone())
            .expect("a frame holds width × height RGBA pixels")
    }
}

/// The pixel format as the status row says it.
pub fn colour_label(colour: ColorType) -> String {
    let (channels, bits) = match colour {
        ColorType::L8 => ("greyscale", 8),
        ColorType::La8 => ("greyscale with alpha", 8),
        ColorType::Rgb8 => ("RGB", 8),
        ColorType::Rgba8 => ("RGBA", 8),
        ColorType::L16 => ("greyscale", 16),
        ColorType::La16 => ("greyscale with alpha", 16),
        ColorType::Rgb16 => ("RGB", 16),
        ColorType::Rgba16 => ("RGBA", 16),
        ColorType::Rgb32F => ("RGB", 32),
        ColorType::Rgba32F => ("RGBA", 32),
        _ => ("pixels", colour.bits_per_pixel() as u16 / colour.channel_count() as u16),
    };
    if colour.channel_count() == 1 {
        format!("{channels}, {bits} bits")
    } else {
        format!("{channels}, {bits} bits per channel")
    }
}

/// Decode a raster file: every frame of an animated GIF, APNG or WebP, one frame
/// otherwise, plus the pixel format and EXIF fields. The format comes from the bytes'
/// signature, or, for formats without one (TGA), from `extension`.
pub fn decode(bytes: &[u8], extension: &str) -> Result<Picture, String> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    if reader.format().is_none()
        && let Some(by_extension) = ImageFormat::from_extension(extension.to_ascii_lowercase())
    {
        reader.set_format(by_extension);
    }
    let format = reader.format().ok_or_else(|| "not an image format the viewer decodes".to_string())?;
    let exif = exif_fields(bytes);
    let animated = match format {
        ImageFormat::Gif => {
            let decoder = GifDecoder::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
            let facts = facts(&decoder);
            Some((facts, frames_of(decoder.into_frames())?))
        }
        ImageFormat::Png => {
            let decoder = PngDecoder::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
            if decoder.is_apng().map_err(|e| e.to_string())? {
                let facts = facts(&decoder);
                let apng = decoder.apng().map_err(|e| e.to_string())?;
                Some((facts, frames_of(apng.into_frames())?))
            } else {
                None
            }
        }
        ImageFormat::WebP => {
            let decoder = WebPDecoder::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
            if decoder.has_animation() {
                let facts = facts(&decoder);
                Some((facts, frames_of(decoder.into_frames())?))
            } else {
                None
            }
        }
        _ => None,
    };
    if let Some(((colour, width, height), frames)) = animated {
        return Ok(Picture { width, height, colour, frames, exif });
    }
    let decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let colour = colour_label(decoder.color_type());
    let image = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    let (width, height) = (image.width(), image.height());
    let rgba = image.into_rgba8().into_raw();
    Ok(Picture { width, height, colour, frames: vec![Frame { rgba, delay_ms: 0 }], exif })
}

/// The pixel format and dimensions a decoder reports.
fn facts(decoder: &impl ImageDecoder) -> (String, u32, u32) {
    let (width, height) = decoder.dimensions();
    (colour_label(decoder.color_type()), width, height)
}

/// Every frame of an animation with its delay.
fn frames_of(frames: image::Frames<'_>) -> Result<Vec<Frame>, String> {
    let frames = frames.collect_frames().map_err(|e| e.to_string())?;
    if frames.is_empty() {
        return Err("the animation has no frames".into());
    }
    Ok(frames
        .into_iter()
        .map(|frame| {
            let (numer, denom) = frame.delay().numer_denom_ms();
            let delay_ms = if denom == 0 { 0 } else { numer / denom };
            Frame { rgba: frame.into_buffer().into_raw(), delay_ms }
        })
        .collect())
}

/// The EXIF fields shown, in this order, when the file carries them.
const EXIF_TAGS: [(exif::Tag, &str); 10] = [
    (exif::Tag::Make, "Camera make"),
    (exif::Tag::Model, "Camera model"),
    (exif::Tag::DateTimeOriginal, "Taken"),
    (exif::Tag::ExposureTime, "Exposure"),
    (exif::Tag::FNumber, "Aperture"),
    (exif::Tag::PhotographicSensitivity, "ISO"),
    (exif::Tag::FocalLength, "Focal length"),
    (exif::Tag::Orientation, "Orientation"),
    (exif::Tag::Software, "Software"),
    (exif::Tag::ImageDescription, "Description"),
];

/// The EXIF fields of `EXIF_TAGS` present in the file, as the EXIF crate displays them.
pub fn exif_fields(bytes: &[u8]) -> Vec<(String, String)> {
    let Ok(data) = exif::Reader::new().read_from_container(&mut Cursor::new(bytes)) else {
        return Vec::new();
    };
    EXIF_TAGS
        .iter()
        .filter_map(|(tag, name)| {
            let field = data.get_field(*tag, exif::In::PRIMARY)?;
            let value = field.display_value().with_unit(&data).to_string();
            (!value.is_empty()).then(|| (name.to_string(), value))
        })
        .collect()
}

/// Unpack an svgz; plain SVG bytes pass through.
pub fn svg_bytes(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(bytes).read_to_end(&mut out).map_err(|e| e.to_string())?;
        Ok(out)
    } else {
        Ok(bytes.to_vec())
    }
}

/// The family name a TrueType font declares, and its glyph count.
pub fn font_facts(bytes: &[u8]) -> Result<(String, u16), String> {
    let face = ttf_parser::Face::parse(bytes, 0).map_err(|e| format!("not a font the viewer reads: {e}"))?;
    let family = face
        .names()
        .into_iter()
        .filter(|n| n.name_id == ttf_parser::name_id::FAMILY && n.is_unicode())
        .find_map(|n| n.to_string())
        .unwrap_or_else(|| "unnamed family".to_string());
    Ok((family, face.number_of_glyphs()))
}

/// A stable checksum of pixels (FNV-1a), for tests and the status row.
pub fn checksum(rgba: &[u8]) -> u64 {
    rgba.iter().fold(0xcbf2_9ce4_8422_2325u64, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The first frame encoded in the format the extension names. JPEG and GIF lose
/// information; the formats in `LOSSLESS` do not.
pub fn encode(picture: &Picture, extension: &str) -> Result<Vec<u8>, String> {
    let extension = extension.to_ascii_lowercase();
    if !WRITABLE.contains(&extension.as_str()) {
        return Err(format!("`.{extension}` is not a format the viewer writes; use one of {}", WRITABLE.join(", ")));
    }
    let format = ImageFormat::from_extension(&extension)
        .ok_or_else(|| format!("`.{extension}` names no image format"))?;
    let image = DynamicImage::ImageRgba8(picture.first());
    let image = match format {
        // These encoders take no alpha channel.
        ImageFormat::Jpeg => DynamicImage::ImageRgb8(image.to_rgb8()),
        ImageFormat::Hdr => DynamicImage::ImageRgb32F(image.to_rgb32f()),
        ImageFormat::OpenExr => DynamicImage::ImageRgba32F(image.to_rgba32f()),
        // Farbfeld is 16 bits per channel; 8-bit values scale up and back exactly.
        ImageFormat::Farbfeld => DynamicImage::ImageRgba16(image.to_rgba16()),
        ImageFormat::Pnm if extension != "pam" => DynamicImage::ImageRgb8(image.to_rgb8()),
        _ => image,
    };
    let mut out = Cursor::new(Vec::new());
    image.write_to(&mut out, format).map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

/// Write the first frame to `path` in the format its extension names.
pub fn save_as(picture: &Picture, path: &std::path::Path) -> Result<usize, String> {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .ok_or_else(|| "the file name needs an extension to choose the format".to_string())?;
    let bytes = encode(picture, extension)?;
    std::fs::write(path, &bytes).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(bytes.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::codecs::gif::{GifEncoder, Repeat};
    use image::{Delay, Rgba};

    /// An 8 × 6 pattern with every channel in play, including transparency.
    fn pattern() -> RgbaImage {
        RgbaImage::from_fn(8, 6, |x, y| {
            Rgba([(x * 32) as u8, (y * 40) as u8, ((x + y) * 17) as u8, if x < 4 { 255 } else { 128 }])
        })
    }

    fn encoded(format: ImageFormat) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        let image = DynamicImage::ImageRgba8(pattern());
        let image = if format == ImageFormat::Farbfeld { DynamicImage::ImageRgba16(image.to_rgba16()) } else { image };
        image.write_to(&mut out, format).unwrap();
        out.into_inner()
    }

    #[test]
    fn every_listed_extension_has_a_kind_and_the_rest_are_named_unsupported() {
        for ext in RASTER {
            assert_eq!(kind_of(&format!("x.{ext}")), Some(Kind::Raster), "{ext}");
            assert_eq!(kind_of(&format!("X.{}", ext.to_uppercase())), Some(Kind::Raster), "{ext}");
        }
        for ext in VECTOR {
            assert_eq!(kind_of(&format!("x.{ext}")), Some(Kind::Vector), "{ext}");
        }
        assert_eq!(kind_of("font.ttf"), Some(Kind::Font));
        for ext in UNSUPPORTED {
            assert_eq!(kind_of(&format!("x.{ext}")), None, "{ext} has no decoder yet");
        }
        assert_eq!(kind_of("noext"), None);
        // The family's extension table and these lists agree exactly.
        let family: std::collections::BTreeSet<&str> = crate::generated::families::EXTENSIONS
            .iter()
            .filter(|(_, families)| families.contains(&crate::Family::Images))
            .map(|(ext, _)| *ext)
            .collect();
        let listed: std::collections::BTreeSet<&str> =
            RASTER.iter().chain(VECTOR).chain(FONT).chain(UNSUPPORTED).copied().collect();
        let extra: Vec<_> = listed.difference(&family).collect();
        let missing: Vec<_> = family.difference(&listed).collect();
        assert!(extra.is_empty() && missing.is_empty(), "listed but not in the family: {extra:?}; in the family but unlisted: {missing:?}");
        for ext in LOSSLESS {
            assert!(WRITABLE.contains(ext), "{ext}");
        }
    }

    #[test]
    fn raster_files_decode_to_exact_pixels_and_metadata() {
        let expected = pattern().into_raw();
        for (format, colour) in [
            (ImageFormat::Png, "RGBA, 8 bits per channel"),
            (ImageFormat::Bmp, "RGBA, 8 bits per channel"),
            (ImageFormat::Tiff, "RGBA, 8 bits per channel"),
            (ImageFormat::Tga, "RGBA, 8 bits per channel"),
            (ImageFormat::Qoi, "RGBA, 8 bits per channel"),
            (ImageFormat::Farbfeld, "RGBA, 16 bits per channel"),
            (ImageFormat::WebP, "RGBA, 8 bits per channel"),
        ] {
            let picture = decode(&encoded(format), format.extensions_str()[0]).unwrap_or_else(|e| panic!("{format:?}: {e}"));
            assert_eq!((picture.width, picture.height), (8, 6), "{format:?}");
            assert_eq!(picture.colour, colour, "{format:?}");
            assert_eq!(picture.frames.len(), 1, "{format:?}");
            assert_eq!(picture.frames[0].rgba, expected, "{format:?} pixels");
            assert_eq!(picture.frames[0].delay_ms, 0);
            assert!(!picture.is_animated());
            assert!(picture.exif.is_empty());
        }
        // JPEG is lossy: same size, no alpha, pixels close to the pattern.
        let jpeg = decode(&encoded(ImageFormat::Jpeg), "jpg").unwrap();
        assert_eq!((jpeg.width, jpeg.height, jpeg.colour.as_str()), (8, 6, "RGB, 8 bits per channel"));
        let drift: u32 = jpeg.frames[0].rgba.chunks(4).zip(expected.chunks(4))
            .map(|(a, b)| a[..3].iter().zip(&b[..3]).map(|(x, y)| x.abs_diff(*y) as u32).sum::<u32>())
            .sum();
        assert!(drift < 8 * 6 * 3 * 40, "jpeg drift {drift}");
        assert!(jpeg.frames[0].rgba.chunks(4).all(|p| p[3] == 255), "no alpha in JPEG");
        // Greyscale is named as such.
        let mut grey = Cursor::new(Vec::new());
        DynamicImage::ImageLuma8(DynamicImage::ImageRgba8(pattern()).to_luma8()).write_to(&mut grey, ImageFormat::Png).unwrap();
        assert_eq!(decode(&grey.into_inner(), "png").unwrap().colour, "greyscale, 8 bits");
        assert_eq!(colour_label(ColorType::Rgb16), "RGB, 16 bits per channel");
        // The checksum is stable and tells frames apart.
        assert_eq!(checksum(&expected), checksum(&pattern().into_raw()));
        assert_ne!(checksum(&expected), checksum(&jpeg.frames[0].rgba));
        assert_eq!(checksum(b""), 0xcbf2_9ce4_8422_2325);
        // Not an image.
        assert!(decode(b"not an image at all", "txt").is_err());
        assert!(decode(&[], "").is_err());
    }

    #[test]
    fn animated_gifs_decode_every_frame_with_its_delay() {
        let mut bytes = Cursor::new(Vec::new());
        {
            let mut encoder = GifEncoder::new(&mut bytes);
            encoder.set_repeat(Repeat::Infinite).unwrap();
            let first = image::Frame::from_parts(pattern(), 0, 0, Delay::from_numer_denom_ms(100, 1));
            let mut second_pixels = pattern();
            for p in second_pixels.pixels_mut() { p.0[0] = 255 - p.0[0]; }
            let second = image::Frame::from_parts(second_pixels, 0, 0, Delay::from_numer_denom_ms(250, 1));
            encoder.encode_frames([first, second]).unwrap();
        }
        let picture = decode(&bytes.into_inner(), "gif").unwrap();
        assert_eq!((picture.width, picture.height), (8, 6));
        assert_eq!(picture.frames.len(), 2);
        assert!(picture.is_animated());
        assert_eq!(picture.frames[0].delay_ms, 100);
        assert_eq!(picture.frames[1].delay_ms, 250);
        assert_ne!(picture.frames[0].rgba, picture.frames[1].rgba);
        assert_eq!(picture.frames[0].rgba.len(), 8 * 6 * 4);
        // A still PNG has one frame; the fixture dot.png is 1 × 1.
        let dot = decode(&std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/dot.png")).unwrap(), "png").unwrap();
        assert_eq!((dot.width, dot.height, dot.frames.len()), (1, 1, 1));
    }

    #[test]
    fn save_as_round_trips_lossless_formats_and_refuses_the_rest() {
        let picture = decode(&encoded(ImageFormat::Png), "png").unwrap();
        for ext in LOSSLESS {
            let bytes = encode(&picture, ext).unwrap_or_else(|e| panic!("{ext}: {e}"));
            let back = decode(&bytes, ext).unwrap_or_else(|e| panic!("{ext}: {e}"));
            assert_eq!((back.width, back.height), (8, 6), "{ext}");
            assert_eq!(back.frames[0].rgba, picture.frames[0].rgba, "{ext} is lossless");
        }
        // Writable but not lossless: the result still decodes at the same size.
        for ext in ["jpg", "gif", "ico", "pnm", "exr", "hdr"] {
            let bytes = encode(&picture, ext).unwrap_or_else(|e| panic!("{ext}: {e}"));
            let back = decode(&bytes, ext).unwrap_or_else(|e| panic!("{ext}: {e}"));
            assert_eq!((back.width, back.height), (8, 6), "{ext}");
        }
        assert!(encode(&picture, "svg").unwrap_err().starts_with("`.svg` is not a format the viewer writes"));
        assert!(encode(&picture, "xyz").is_err());
        // Through a path.
        let dir = std::env::temp_dir();
        let path = dir.join(format!("sfoglia-image-{}.bmp", std::process::id()));
        let written = save_as(&picture, &path).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len() as usize, written);
        assert_eq!(decode(&std::fs::read(&path).unwrap(), "bmp").unwrap().frames[0].rgba, picture.frames[0].rgba);
        let _ = std::fs::remove_file(&path);
        assert!(save_as(&picture, &dir.join("no-extension")).unwrap_err().contains("needs an extension"));
    }

    #[test]
    fn svgz_unpacks_and_fonts_name_their_family() {
        let svg = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/dot.svg")).unwrap();
        assert_eq!(svg_bytes(&svg).unwrap(), svg, "plain SVG passes through");
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut gz, &svg).unwrap();
        let svgz = gz.finish().unwrap();
        assert_eq!(svg_bytes(&svgz).unwrap(), svg, "svgz unpacks to the same SVG");
        assert!(font_facts(b"not a font").is_err());
        if let Some(font) = system_font() {
            let (family, glyphs) = font_facts(&std::fs::read(font).unwrap()).unwrap();
            assert!(!family.is_empty() && family != "unnamed family", "{family}");
            assert!(glyphs > 100, "{family} has {glyphs} glyphs");
        }
    }

    /// A TrueType font the test machine has; none on a machine without these.
    pub fn system_font() -> Option<std::path::PathBuf> {
        ["C:/Windows/Fonts/arial.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", "/usr/share/fonts/TTF/DejaVuSans.ttf"]
            .into_iter()
            .map(std::path::PathBuf::from)
            .find(|p| p.exists())
    }
}

/// The size an SVG declares, in CSS pixels, and its parsed tree.
pub fn svg_tree(svg: &[u8]) -> Result<usvg::Tree, String> {
    usvg::Tree::from_data(svg, &usvg::Options::default()).map_err(|e| format!("not an SVG the viewer renders: {e}"))
}

/// The SVG rasterized at `zoom` times its own size: straight-alpha RGBA, as every other
/// picture here. `size` is the intrinsic size the view fits and zooms.
pub fn rasterize_svg(tree: &usvg::Tree, zoom: f32) -> Result<Picture, String> {
    let size = tree.size();
    let (w, h) = ((size.width() * zoom).round().max(1.) as u32, (size.height() * zoom).round().max(1.) as u32);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h).ok_or_else(|| format!("cannot rasterize {w} × {h} pixels"))?;
    let sx = w as f32 / size.width();
    let sy = h as f32 / size.height();
    resvg::render(tree, resvg::tiny_skia::Transform::from_scale(sx, sy), &mut pixmap.as_mut());
    let mut rgba = pixmap.take();
    // tiny-skia premultiplies; undo it so transparent pixels keep their colour.
    for pixel in rgba.chunks_exact_mut(4) {
        let a = pixel[3] as u32;
        if a > 0 && a < 255 {
            for c in &mut pixel[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    Ok(Picture {
        width: w,
        height: h,
        colour: "vectors".to_string(),
        frames: vec![Frame { rgba, delay_ms: 0 }],
        exif: Vec::new(),
    })
}

#[cfg(test)]
mod svg_tests {
    use super::*;

    #[test]
    fn svgs_rasterize_at_their_own_size_and_at_a_zoom() {
        let svg = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/dot.svg")).unwrap();
        let tree = svg_tree(&svg).unwrap();
        assert_eq!((tree.size().width(), tree.size().height()), (40., 30.));
        let one = rasterize_svg(&tree, 1.0).unwrap();
        assert_eq!((one.width, one.height, one.frames.len()), (40, 30, 1));
        // The corner is the orange rectangle, the centre the blue circle, both opaque.
        let pixel = |p: &Picture, x: u32, y: u32| {
            let i = ((y * p.width + x) * 4) as usize;
            [p.frames[0].rgba[i], p.frames[0].rgba[i + 1], p.frames[0].rgba[i + 2], p.frames[0].rgba[i + 3]]
        };
        assert_eq!(pixel(&one, 1, 1), [0xff, 0x9f, 0x0a, 0xff]);
        assert_eq!(pixel(&one, 20, 15), [0x1d, 0x4e, 0xd8, 0xff]);
        let two = rasterize_svg(&tree, 2.0).unwrap();
        assert_eq!((two.width, two.height), (80, 60));
        assert_eq!(pixel(&two, 40, 30), [0x1d, 0x4e, 0xd8, 0xff]);
        assert_eq!(pixel(&two, 2, 2), [0xff, 0x9f, 0x0a, 0xff]);
        let tiny = rasterize_svg(&tree, 0.01).unwrap();
        assert_eq!((tiny.width, tiny.height), (1, 1), "never below one pixel");
        assert!(svg_tree(b"<svg").is_err());
        assert!(svg_tree(b"not svg").is_err());
    }
}
