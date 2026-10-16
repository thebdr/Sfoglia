//! Headless tests of the image viewer (C-009): a real gpui-kit window opens raster files
//! fitted with their facts, zooms and rotates them, saves them as another format, and
//! tells vectors, fonts, unsupported extensions and corrupt files apart.

use std::io::Cursor;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, Focusable as _, ScrollDelta, TestAppContext, point, px};
use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
use sfoglia::images;
use sfoglia::viewers::image_view::Shown;
use sfoglia::{Workspace, window_options};

fn open_workspace(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Workspace>) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let rules_path = std::env::temp_dir().join(format!("sfoglia-image-rules-{}-{n}.toml", std::process::id()));
    cx.update(gpui_kit::init);
    cx.update(sfoglia::init);
    cx.update(|cx| {
        gpui_kit::open_window(window_options(), cx, |window, cx| {
            cx.new(|cx| Workspace::with_rules_path(Some(rules_path.clone()), window, cx))
        })
        .expect("open the main window headlessly")
    })
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(name)
}

fn scratch_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sfoglia-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A pattern with every channel in play, written in `format`.
fn pattern(width: u32, height: u32) -> RgbaImage {
    RgbaImage::from_fn(width, height, |x, y| {
        Rgba([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8, if x % 2 == 0 { 255 } else { 96 }])
    })
}

fn write_image(path: &std::path::Path, image: &RgbaImage, format: ImageFormat) {
    let mut out = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image.clone()).write_to(&mut out, format).unwrap();
    std::fs::write(path, out.into_inner()).unwrap();
}

/// A wide PNG opens fitted with its facts; 1, +, -, 0 and R change zoom and rotation as
/// stated; the view's save writes a lossless copy; a tiny image fits at 100 %; an
/// animated GIF counts its frames.
#[gpui_kit::test]
fn images_open_fitted_zoom_rotate_and_show_their_facts(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    let dir = scratch_dir("images");
    let wide = dir.join("wide.png");
    let wide_pixels = pattern(2000, 400);
    write_image(&wide, &wide_pixels, ImageFormat::Png);
    let file_len = std::fs::metadata(&wide).unwrap().len();

    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |w, cx| w.open_paths(&[wide.clone()], window, cx));
        window.render_frame(cx);
        // The first frame measured the picture area; the second lays the image out to fit it.
        window.render_frame(cx);
        let panel = workspace.read(cx).panels(cx)[0].clone();
        let view = panel.read(cx).image_view().expect("a PNG opens in the image viewer").clone();
        assert!(panel.read(cx).text_view().is_none());
        assert!(window.try_find("file-viewer").is_none(), "the info rows give way to the image");
        let picture = view.read(cx).picture().expect("a decoded raster").clone();
        assert_eq!((picture.width, picture.height), (2000, 400));
        assert_eq!(picture.frames[0].rgba, wide_pixels.as_raw().as_slice());
        assert_eq!(window.find("image-size").label(), Some("2000 × 400 pixels"));
        assert_eq!(window.find("image-colour").label(), Some("RGBA, 8 bits per channel"));
        assert_eq!(window.find("image-frames").label(), Some("still"));
        assert_eq!(window.find("image-rotation").label(), Some("0°"));
        assert_eq!(window.find("image-bytes").label(), Some(format!("{file_len} bytes").as_str()));
        assert!(window.try_find("image-exif").is_none(), "a generated PNG carries no EXIF");
        // Fitted: the zoom follows the measured area and never goes above 100 %.
        let viewport = view.read(cx).viewport().expect("the picture area was measured");
        assert!(viewport.width > px(300.) && viewport.height > px(300.), "{viewport:?}");
        let fit = (f32::from(viewport.width) / 2000.).min(f32::from(viewport.height) / 400.).min(1.0);
        assert_eq!(view.read(cx).zoom(), None);
        assert!((view.read(cx).effective_zoom() - fit).abs() < 1e-5, "{} vs {fit}", view.read(cx).effective_zoom());
        assert!(fit < 1.0, "the 2000-pixel image must shrink to fit: {fit}");
        let zoom_label = window.find("image-zoom").label().unwrap_or_default().to_string();
        assert_eq!(zoom_label, format!("fit, {} %", (fit * 100.).round() as u32));
        let box_width = f32::from(window.find("image-box").bounds().size.width);
        assert!((box_width - 2000. * fit).abs() <= 1.5, "box {box_width} vs {}", 2000. * fit);

        // The dock handed the focus to the view, so the keys work at once: 1 shows the
        // image at 100 %, wider than the area, so it pans by scrolling.
        assert!(view.read(cx).focus_handle(cx).is_focused(window), "the image view holds focus when its tab shows");
        window.press("1", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).zoom(), Some(1.0));
        assert_eq!(window.find("image-zoom").label(), Some("100 %"));
        let box_width = f32::from(window.find("image-box").bounds().size.width);
        assert!((box_width - 2000.).abs() <= 1.5, "box {box_width}");
        assert!(box_width > f32::from(viewport.width), "the image is larger than its area and pans");
        assert_eq!(view.read(cx).scroll_offset().x, px(0.));
        window.scroll("image-scroll", ScrollDelta::Lines(point(-5., 0.)), cx);
        window.render_frame(cx);
        assert!(view.read(cx).scroll_offset().x < px(0.), "the wheel pans the image: {:?}", view.read(cx).scroll_offset());
        // + and - step by a quarter; 0 fits again.
        window.press("+", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).zoom(), Some(1.25));
        assert_eq!(window.find("image-zoom").label(), Some("125 %"));
        window.press("-", cx);
        window.render_frame(cx);
        assert!((view.read(cx).zoom().unwrap() - 1.0).abs() < 1e-6);
        window.press("0", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).zoom(), None);
        assert!(window.find("image-zoom").label().unwrap_or_default().starts_with("fit, "));
        // R turns the image a quarter: the drawn size swaps, the file's size stays.
        window.press("r", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).rotation(), 1);
        assert_eq!(view.read(cx).drawn_size(), Some((400, 2000)));
        assert_eq!(window.find("image-rotation").label(), Some("90°, drawn 400 × 2000"));
        assert_eq!(window.find("image-size").label(), Some("2000 × 400 pixels"));
        window.render_frame(cx);
        let bounds = window.find("image-box").bounds();
        let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
        assert!((w / h - 400. / 2000.).abs() < 0.01, "the box is tall now: {w} × {h}");
        // The frames handed to the window are turned, not stretched: 400 × 2000, with the
        // source's bottom-left pixel (0, 399) now at the top-left, in BGRA.
        assert_eq!(view.read(cx).rendered_size(), Some((400, 2000)));
        let source = |x: u32, y: u32| {
            let p = wide_pixels.get_pixel(x, y).0;
            [p[2], p[1], p[0], p[3]]
        };
        assert_eq!(view.read(cx).rendered_pixel(0, 0), Some(source(0, 399)));
        assert_eq!(view.read(cx).rendered_pixel(399, 0), Some(source(0, 0)));
        assert_eq!(view.read(cx).rendered_pixel(0, 1999), Some(source(1999, 399)));
        // Two more turns: 180°, the same size as the file, every pixel at its opposite place.
        window.press("r", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).rendered_size(), Some((2000, 400)));
        assert_eq!(view.read(cx).rendered_pixel(1999, 399), Some(source(0, 0)));
        assert_eq!(view.read(cx).rendered_pixel(0, 0), Some(source(1999, 399)));
        assert_eq!(window.find("image-rotation").label(), Some("180°, drawn 2000 × 400"));
        window.press("r", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).rendered_size(), Some((400, 2000)));
        assert_eq!(view.read(cx).rendered_pixel(0, 0), Some(source(1999, 0)), "270°: the top-right comes first");
        window.press("r", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).rotation(), 0);
        assert_eq!(view.read(cx).rendered_size(), Some((2000, 400)));
        assert_eq!(view.read(cx).rendered_pixel(0, 0), Some(source(0, 0)));
        assert_eq!(window.find("image-rotation").label(), Some("0°"));
        // The view's save writes a lossless BMP with identical pixels.
        let copy = dir.join("copy.bmp");
        let written = view.update(cx, |view, cx| view.save_as(&copy, cx)).unwrap();
        assert_eq!(std::fs::metadata(&copy).unwrap().len() as usize, written);
        let back = images::decode(&std::fs::read(&copy).unwrap(), "bmp").unwrap();
        assert_eq!(back.frames[0].rgba, wide_pixels.as_raw().as_slice());
        window.render_frame(cx);
        let message = window.find("image-message").label().unwrap_or_default().to_string();
        assert!(message.starts_with("saved "), "{message}");
        assert!(view.update(cx, |view, cx| view.save_as(&dir.join("copy.svg"), cx)).is_err());

        // A 1 × 1 PNG fits at 100 %: fitting never enlarges.
        workspace.update(cx, |w, cx| w.open_paths(&[fixture("dot.png")], window, cx));
        window.render_frame(cx);
        window.render_frame(cx);
        let dot = workspace.read(cx).panels(cx)[1].read(cx).image_view().unwrap().clone();
        assert_eq!(dot.read(cx).drawn_size(), Some((1, 1)));
        assert_eq!(window.find("image-zoom").label(), Some("fit, 100 %"));
        assert_eq!(window.find("image-size").label(), Some("1 × 1 pixels"));
        assert_eq!(window.find("image-bytes").label(), Some("67 bytes"));
        // Ctrl+Shift+S asks for a new name through the save dialog.
        assert!(dot.read(cx).focus_handle(cx).is_focused(window));
        window.press("ctrl-shift-s", cx);
    })
    .unwrap();
    let chord_copy = dir.join("chord.png");
    cx.simulate_new_path_selection(|_| Some(chord_copy.clone()));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let back = images::decode(&std::fs::read(&chord_copy).unwrap(), "png").unwrap();
        assert_eq!((back.width, back.height), (1, 1), "the dialog's name was written");
        assert_eq!(back.frames[0].rgba, images::decode(&std::fs::read(fixture("dot.png")).unwrap(), "png").unwrap().frames[0].rgba);
        assert!(window.find("image-message").label().unwrap_or_default().starts_with("saved "));
        // A JPEG with EXIF: the fields in the stated order, one row each.
        workspace.update(cx, |w, cx| w.open_paths(&[fixture("exif.jpg")], window, cx));
        window.render_frame(cx);
        let photo = workspace.read(cx).panels(cx)[2].read(cx).image_view().unwrap().clone();
        assert_eq!(photo.read(cx).picture().unwrap().exif.len(), 10);
        assert_eq!(window.find(("image-exif", 0usize)).label(), Some("Camera make: Sfoglia Cam"));
        assert_eq!(window.find(("image-exif", 1usize)).label(), Some("Camera model: Model Nine"));
        assert_eq!(window.find(("image-exif", 3usize)).label(), Some("Exposure: 1/250 s"));
        assert_eq!(window.find(("image-exif", 4usize)).label(), Some("Aperture: f/2.8"));
        assert_eq!(window.find(("image-exif", 9usize)).label(), Some("Description: A test frame"));
        assert!(window.try_find(("image-exif", 10usize)).is_none());
        assert_eq!(window.find("image-colour").label(), Some("RGB, 8 bits per channel"));
    })
    .unwrap();

    // An animated GIF: two frames.
    let animated = dir.join("blink.gif");
    {
        use image::codecs::gif::GifEncoder;
        let mut bytes = Cursor::new(Vec::new());
        let mut encoder = GifEncoder::new(&mut bytes);
        let first = image::Frame::from_parts(pattern(16, 16), 0, 0, image::Delay::from_numer_denom_ms(80, 1));
        let second = image::Frame::from_parts(pattern(16, 16), 0, 0, image::Delay::from_numer_denom_ms(120, 1));
        encoder.encode_frames([first, second]).unwrap();
        drop(encoder);
        std::fs::write(&animated, bytes.into_inner()).unwrap();
    }
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |w, cx| w.open_paths(&[animated.clone()], window, cx));
        window.render_frame(cx);
        let view = workspace.read(cx).panels(cx)[3].read(cx).image_view().unwrap().clone();
        assert_eq!(view.read(cx).picture().unwrap().frames.len(), 2);
        assert_eq!(window.find("image-frames").label(), Some("2 frames"));
        assert_eq!(window.find("image-size").label(), Some("16 × 16 pixels"));
        // Both frames, with their delays, reach the window's image, which plays them.
        assert_eq!(view.read(cx).rendered_frame_delays(), [80, 120]);
        let still = workspace.read(cx).panels(cx)[0].read(cx).image_view().unwrap().clone();
        assert_eq!(still.read(cx).rendered_frame_delays().len(), 1);
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// An SVG renders through the window's renderer at its own size, a font shows its family
/// and glyph count in a specimen, an unsupported extension and a corrupt file say so, and
/// an image tab still toggles to the bytes.
#[gpui_kit::test]
fn vectors_fonts_and_unsupported_formats_are_told_apart(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    let dir = scratch_dir("images-kinds");
    let psd = dir.join("layers.psd");
    std::fs::write(&psd, b"8BPS\x00\x01 not really a photoshop file").unwrap();
    let broken = dir.join("broken.png");
    std::fs::write(&broken, b"\x89PNG\r\n\x1a\n this is not a png body").unwrap();
    let font = ["C:/Windows/Fonts/arial.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", "/usr/share/fonts/TTF/DejaVuSans.ttf"]
        .into_iter()
        .map(PathBuf::from)
        .find(|p| p.exists());

    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |w, cx| w.open_paths(&[fixture("dot.svg"), psd.clone(), broken.clone()], window, cx));
        window.render_frame(cx);
        let panels = workspace.read(cx).panels(cx);
        // The SVG: vectors rendered at the file's 40 × 30.
        let svg = panels[0].read(cx).image_view().expect("an SVG opens in the image viewer").clone();
        assert!(matches!(svg.read(cx).shown(), Shown::Vector { .. }));
        assert_eq!(svg.read(cx).drawn_size(), Some((40, 30)));
        assert!(svg.read(cx).picture().is_none());
        // The unsupported extension and the corrupt file explain themselves.
        let psd_view = panels[1].read(cx).image_view().unwrap().clone();
        assert!(matches!(psd_view.read(cx).shown(), Shown::Unsupported));
        let broken_view = panels[2].read(cx).image_view().unwrap().clone();
        assert!(matches!(broken_view.read(cx).shown(), Shown::Failed(_)));
        let note = window.find("image-note").label().unwrap_or_default().to_string();
        assert!(note.starts_with("Cannot decode this file: "), "{note}");
        assert!(note.len() > "Cannot decode this file: ".len() + 8, "the reason is spelled out: {note}");
        // A broken SVG and a broken font fail the same way, each with its own reason.
        let bad_svg = dir.join("bad.svg");
        std::fs::write(&bad_svg, b"<svg xmlns='http://www.w3.org/2000/svg'><rect").unwrap();
        let bad_font = dir.join("bad.ttf");
        std::fs::write(&bad_font, b"not a font at all").unwrap();
        workspace.update(cx, |w, cx| w.open_paths(&[bad_svg.clone()], window, cx));
        window.render_frame(cx);
        assert!(matches!(workspace.read(cx).panels(cx)[3].read(cx).image_view().unwrap().read(cx).shown(), Shown::Failed(_)));
        assert!(window.find("image-note").label().unwrap_or_default().starts_with("Cannot decode this file: not an SVG the viewer renders"));
        workspace.update(cx, |w, cx| w.open_paths(&[bad_font.clone()], window, cx));
        window.render_frame(cx);
        assert!(matches!(workspace.read(cx).panels(cx)[4].read(cx).image_view().unwrap().read(cx).shown(), Shown::Failed(_)));
        assert!(window.find("image-note").label().unwrap_or_default().starts_with("Cannot decode this file: not a font the viewer reads"));
        workspace.update(cx, |w, cx| w.open_paths(&[psd.clone()], window, cx));
        window.render_frame(cx);
        assert_eq!(
            window.find("image-note").label(),
            Some("No decoder for .psd yet; Ctrl+Shift+B shows the bytes.")
        );
        // The SVG tab again, with its facts.
        workspace.update(cx, |w, cx| w.open_paths(&[fixture("dot.svg")], window, cx));
        window.render_frame(cx);
        window.render_frame(cx);
        assert_eq!(window.find("image-size").label(), Some("40 × 30 pixels, from vectors"));
        assert_eq!(window.find("image-zoom").label(), Some("fit, 100 %"));
        assert!(window.try_find("image-colour").is_none());
        // Ctrl+Shift+B shows the bytes of an image tab too, and back.
        let focus = svg.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.press("ctrl-shift-b", cx);
        window.render_frame(cx);
        assert!(panels[0].read(cx).hex_view().is_some());
        assert!(panels[0].read(cx).image_view().is_none());
        assert!(window.find(("hex-row", 0usize)).label().unwrap_or_default().starts_with("00000000  3c 73 76 67"));
        window.press("ctrl-shift-b", cx);
        window.render_frame(cx);
        assert!(panels[0].read(cx).image_view().is_some());
        // Back from the bytes, the view has the focus again: the keys work without a click.
        window.press("1", cx);
        window.render_frame(cx);
        assert_eq!(svg.read(cx).zoom(), Some(1.0));
        assert_eq!(window.find("image-zoom").label(), Some("100 %"));
        assert_eq!(svg.read(cx).rendered_size(), Some((40, 30)));
        assert_eq!(svg.read(cx).rendered_zoom(), Some(1.0));
        // Zooming vectors rasterizes them again at the zoom in effect, so they stay sharp:
        // 125 % is 50 × 38 pixels with the circle's exact colour at the centre.
        window.press("+", cx);
        window.render_frame(cx);
        assert_eq!(svg.read(cx).zoom(), Some(1.25));
        assert_eq!(svg.read(cx).rendered_zoom(), Some(1.25));
        assert_eq!(svg.read(cx).rendered_size(), Some((50, 38)));
        assert_eq!(svg.read(cx).rendered_pixel(25, 19), Some([0xd8, 0x4e, 0x1d, 0xff]), "the blue circle, in BGRA");
        assert_eq!(svg.read(cx).rendered_pixel(1, 1), Some([0x0a, 0x9f, 0xff, 0xff]), "the orange corner, in BGRA");
        window.press("+", cx);
        window.render_frame(cx);
        assert_eq!(svg.read(cx).rendered_size(), Some((63, 47)), "156 %: 62.5 × 46.9 rounded");
        // Zooming out below the file's size keeps the file-size raster, scaled down on screen.
        window.press("0", cx);
        window.render_frame(cx);
        assert_eq!(svg.read(cx).rendered_zoom(), Some(1.0));
        assert_eq!(svg.read(cx).rendered_size(), Some((40, 30)));
    })
    .unwrap();

    if let Some(font) = font {
        let (family, glyphs) = images::font_facts(&std::fs::read(&font).unwrap()).unwrap();
        cx.update_window(handle, |_, window, cx| {
            workspace.update(cx, |w, cx| w.open_paths(&[font.clone()], window, cx));
            window.render_frame(cx);
            let view = workspace.read(cx).panels(cx)[5].read(cx).image_view().expect("a font opens in the image viewer").clone();
            assert!(matches!(view.read(cx).shown(), Shown::Font { .. }));
            assert_eq!(window.find("image-font-family").label(), Some(family.as_str()));
            assert_eq!(window.find("image-font-glyphs").label(), Some(format!("{glyphs} glyphs").as_str()));
            assert_eq!(window.find("image-specimen").label(), Some(family.as_str()));
            assert!(window.try_find("image-size").is_none());
        })
        .unwrap();
    } else {
        println!("no system TrueType font found; the specimen part did not run");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
