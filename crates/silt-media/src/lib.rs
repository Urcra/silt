//! Images and videos (ADR 0120): what a file is from its first bytes, an
//! image's size, and the smaller copies the conversation and the viewer
//! show before the original.

pub mod range;
#[cfg(feature = "serve")]
pub mod serve;

use std::io::{BufRead, BufReader, Cursor, Seek};
use std::path::Path;

use image::codecs::jpeg::JpegEncoder;
use image::codecs::webp::WebPEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};

/// How many bytes [`sniff`] needs to tell any kind it knows.
pub const HEAD: usize = 1024;

/// The long edges of an image's previews: the conversation's, drawn at
/// 420 px on a screen of twice the density, and the viewer's first look.
pub const PREVIEW_EDGES: [u32; 2] = [960, 2048];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Image,
    Video,
}

/// A file's media type and kind, from its first bytes; `None` for anything
/// that isn't an image or a video Silt shows.
pub fn sniff(head: &[u8]) -> Option<(&'static str, Kind)> {
    let image = |media_type| Some((media_type, Kind::Image));
    let video = |media_type| Some((media_type, Kind::Video));
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        return image("image/png");
    }
    if head.starts_with(&[0xff, 0xd8, 0xff]) {
        return image("image/jpeg");
    }
    if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        return image("image/gif");
    }
    if head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        return image("image/webp");
    }
    if head.len() >= 12 && &head[4..8] == b"ftyp" {
        // ISO media's brands also name still images (HEIF, AVIF) and sound.
        return match &head[8..12] {
            b"qt  " => video("video/quicktime"),
            b"avif" | b"avis" | b"heic" | b"heix" | b"heim" | b"heis" | b"mif1" | b"msf1" | b"M4A " | b"M4B " | b"M4P " => None,
            _ => video("video/mp4"),
        };
    }
    if head.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        let webm = head.windows(4).take(64).any(|window| window == b"webm");
        return video(if webm { "video/webm" } else { "video/x-matroska" });
    }
    let text = match std::str::from_utf8(head) {
        Ok(text) => text,
        // The head may end inside a character.
        Err(error) => std::str::from_utf8(&head[..error.valid_up_to()]).unwrap_or_default(),
    };
    looks_like_svg(text).then_some(("image/svg+xml", Kind::Image))
}

/// SVG only as text that says so.
pub fn looks_like_svg(text: &str) -> bool {
    let start: String = text.trim_start_matches('\u{feff}').trim_start().chars().take(1024).collect();
    start.contains("<svg")
}

/// Whether a media type names a video.
pub fn is_video(media_type: &str) -> bool {
    media_type.starts_with("video/")
}

/// A smaller copy of an image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    /// The long edge it was made for, one of [`PREVIEW_EDGES`].
    pub edge: u32,
    pub width: u32,
    pub height: u32,
    pub media_type: &'static str,
    pub bytes: Vec<u8>,
}

/// An image's size as shown, and its previews, smallest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Previewed {
    pub width: u32,
    pub height: u32,
    pub previews: Vec<Preview>,
}

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("the image can't be read: {0}")]
    Image(#[from] image::ImageError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// The decoder's bounds: an image past them gets no previews, and the viewer
/// offers its original instead.
fn limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(32_768);
    limits.max_image_height = Some(32_768);
    limits.max_alloc = Some(1 << 30);
    limits
}

/// The image's width and height as shown, with its orientation applied,
/// read from its header alone; `None` for anything else.
pub fn dimensions(path: &Path) -> Option<(u32, u32)> {
    let reader = ImageReader::open(path).ok()?.with_guessed_format().ok()?;
    let mut decoder = reader.into_decoder().ok()?;
    let (width, height) = decoder.dimensions();
    let turned = decoder.orientation().is_ok_and(|orientation| {
        use image::metadata::Orientation::*;
        matches!(orientation, Rotate90 | Rotate270 | Rotate90FlipH | Rotate270FlipH)
    });
    Some(if turned { (height, width) } else { (width, height) })
}

/// The previews an image at `path` gets: one for each of [`PREVIEW_EDGES`]
/// its long edge is longer than. GIFs and animated WebPs get none, so their
/// animation is what people see; SVGs and videos aren't decoded here.
pub fn previews(path: &Path) -> Result<Previewed, MediaError> {
    previews_of(BufReader::new(std::fs::File::open(path)?), &PREVIEW_EDGES)
}

/// [`previews`] for one edge, as a file page asks for its image's preview.
pub fn preview(path: &Path, edge: u32) -> Result<Option<Preview>, MediaError> {
    Ok(previews_of(BufReader::new(std::fs::File::open(path)?), &[edge])?.previews.pop())
}

fn previews_of(mut reader: impl BufRead + Seek, edges: &[u32]) -> Result<Previewed, MediaError> {
    let animated = animated_webp(&mut reader)?;
    let mut reader = ImageReader::new(reader).with_guessed_format()?;
    reader.limits(limits());
    if matches!(reader.format(), Some(ImageFormat::Gif)) || animated {
        let (width, height) = reader.into_dimensions()?;
        return Ok(Previewed { width, height, previews: Vec::new() });
    }
    let mut decoder = reader.into_decoder()?;
    let orientation = decoder.orientation()?;
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    let (width, height) = (image.width(), image.height());
    let mut previews = Vec::new();
    let mut edges: Vec<u32> = edges.iter().copied().filter(|edge| width.max(height) > *edge).collect();
    // Largest first, each made from the last: a smaller copy of a smaller copy
    // is quicker and looks the same.
    edges.sort_unstable_by(|a, b| b.cmp(a));
    let mut from = image;
    for edge in edges {
        let smaller = from.resize(edge, edge, FilterType::CatmullRom);
        previews.push(encoded(&smaller, edge)?);
        from = smaller;
    }
    previews.reverse();
    Ok(Previewed { width, height, previews })
}

/// Whether the reader holds a WebP with frames: its VP8X chunk's flags say.
fn animated_webp(reader: &mut (impl BufRead + Seek)) -> Result<bool, MediaError> {
    let mut head = [0u8; 21];
    let start = reader.stream_position()?;
    let read = std::io::Read::read_exact(reader, &mut head).is_ok();
    reader.seek(std::io::SeekFrom::Start(start))?;
    Ok(read && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP" && &head[12..16] == b"VP8X" && head[20] & 0x02 != 0)
}

/// JPEG where every pixel is opaque, which most screenshots and photos are;
/// else lossless WebP, which keeps the transparency.
fn encoded(image: &DynamicImage, edge: u32) -> Result<Preview, MediaError> {
    let mut bytes = Vec::new();
    let transparent = image.color().has_alpha() && image.to_rgba8().pixels().any(|pixel| pixel.0[3] < 255);
    let media_type = if transparent {
        let rgba = image.to_rgba8();
        WebPEncoder::new_lossless(&mut Cursor::new(&mut bytes)).encode(
            &rgba,
            rgba.width(),
            rgba.height(),
            image::ExtendedColorType::Rgba8,
        )?;
        "image/webp"
    } else {
        let rgb = image.to_rgb8();
        JpegEncoder::new_with_quality(&mut Cursor::new(&mut bytes), 82).encode_image(&rgb)?;
        "image/jpeg"
    };
    Ok(Preview { edge, width: image.width(), height: image.height(), media_type, bytes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb, Rgba};

    fn png(width: u32, height: u32, alpha: bool) -> Vec<u8> {
        let mut bytes = Vec::new();
        let image = if alpha {
            DynamicImage::ImageRgba8(ImageBuffer::from_fn(width, height, |x, _| Rgba([(x % 255) as u8, 40, 90, 128])))
        } else {
            DynamicImage::ImageRgb8(ImageBuffer::from_fn(width, height, |x, y| Rgb([(x % 255) as u8, (y % 255) as u8, 90])))
        };
        image.write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png).expect("png");
        bytes
    }

    #[test]
    fn files_are_told_apart_by_their_first_bytes() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\nrest"), Some(("image/png", Kind::Image)));
        assert_eq!(sniff(&[0xff, 0xd8, 0xff, 0xe0]), Some(("image/jpeg", Kind::Image)));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some(("image/webp", Kind::Image)));
        assert_eq!(sniff(b"\0\0\0\x20ftypisom\0\0\x02\0"), Some(("video/mp4", Kind::Video)));
        assert_eq!(sniff(b"\0\0\0\x14ftypqt  \0\0\0\0"), Some(("video/quicktime", Kind::Video)));
        assert_eq!(sniff(b"\0\0\0\x1cftypheic\0\0\0\0"), None);
        assert_eq!(sniff(b"\0\0\0\x1cftypM4A \0\0\0\0"), None);
        assert_eq!(sniff(b"\x1a\x45\xdf\xa3\x9f\x42\x86\x81\x01\x42\x82\x84webm"), Some(("video/webm", Kind::Video)));
        assert_eq!(sniff(b"\x1a\x45\xdf\xa3\x9f\x42\x82\x88matroska"), Some(("video/x-matroska", Kind::Video)));
        assert_eq!(sniff(b"\xef\xbb\xbf  <svg xmlns='http://www.w3.org/2000/svg'/>"), Some(("image/svg+xml", Kind::Image)));
        assert_eq!(sniff(b"<html>hello</html>"), None);
        assert_eq!(sniff(b""), None);
    }

    #[test]
    fn a_large_image_gets_both_previews_smallest_first() {
        let made = previews_of(Cursor::new(png(3200, 1800, false)), &PREVIEW_EDGES).expect("previews");
        assert_eq!((made.width, made.height), (3200, 1800));
        let sizes: Vec<_> = made.previews.iter().map(|preview| (preview.edge, preview.width, preview.height)).collect();
        assert_eq!(sizes, [(960, 960, 540), (2048, 2048, 1152)]);
        assert!(made.previews.iter().all(|preview| preview.media_type == "image/jpeg"));
        assert_eq!(sniff(&made.previews[0].bytes).map(|found| found.0), Some("image/jpeg"));
    }

    #[test]
    fn a_middling_image_gets_one_and_a_small_one_none() {
        let made = previews_of(Cursor::new(png(1500, 400, false)), &PREVIEW_EDGES).expect("previews");
        assert_eq!(made.previews.iter().map(|preview| preview.edge).collect::<Vec<_>>(), [960]);
        assert!(previews_of(Cursor::new(png(600, 400, false)), &PREVIEW_EDGES).expect("small").previews.is_empty());
    }

    #[test]
    fn transparency_is_kept_as_webp() {
        let made = previews_of(Cursor::new(png(1200, 300, true)), &PREVIEW_EDGES).expect("previews");
        assert_eq!(made.previews[0].media_type, "image/webp");
        assert_eq!(sniff(&made.previews[0].bytes).map(|found| found.0), Some("image/webp"));
    }

    #[test]
    fn a_gif_gets_no_previews_and_dimensions_read_from_the_header() {
        let mut gif = Vec::new();
        DynamicImage::ImageRgb8(ImageBuffer::from_pixel(1200, 700, Rgb([1, 2, 3])))
            .write_to(&mut Cursor::new(&mut gif), ImageFormat::Gif)
            .expect("gif");
        let made = previews_of(Cursor::new(gif.clone()), &PREVIEW_EDGES).expect("gif");
        assert_eq!((made.width, made.height, made.previews.len()), (1200, 700, 0));
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("a.gif");
        std::fs::write(&path, gif).expect("write");
        assert_eq!(dimensions(&path), Some((1200, 700)));
        assert_eq!(dimensions(dir.path()), None);
    }

    #[test]
    fn something_that_isnt_an_image_fails() {
        assert!(previews_of(Cursor::new(b"not an image".to_vec()), &PREVIEW_EDGES).is_err());
    }
}
