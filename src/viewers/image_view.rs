//! The image viewer (C-009): a raster file as frames on a checkerboard, fitted to the
//! view, with zoom, pan, quarter-turn rotation, animation and a column of facts; SVG
//! through the window's renderer; a TrueType font as a specimen; a note for the
//! extensions without a decoder.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Bounds, Context, ElementId, FocusHandle, Focusable, IntoElement, ObjectFit, Pixels,
    Render, RenderImage, Role, ScrollHandle, SharedString, Size, TestSupportExt, Window, actions,
    canvas, div, img, px, size,
};
use image::{Delay, RgbaImage};

use crate::images::{self, Kind, Picture};

actions!(image_view, [ZoomIn, ZoomOut, ZoomActual, ZoomFit, RotateQuarter]);

/// The key context the image view's actions are bound in.
pub const KEY_CONTEXT: &str = "ImageView";

/// Each zoom step multiplies or divides by this.
const ZOOM_STEP: f32 = 1.25;

/// The side of a checkerboard square.
const CHECKER: f32 = 12.;

/// What the tab shows.
pub enum Shown {
    Raster {
        picture: Picture,
        /// The frames as the window paints them, turned by `rotation`.
        image: Arc<RenderImage>,
        /// Quarter turns clockwise: 0, 1, 2 or 3.
        rotation: u8,
    },
    Vector {
        tree: usvg::Tree,
        /// The size the SVG declares, in pixels.
        size: (u32, u32),
        /// The vectors rasterized at `rendered_zoom` times their size.
        image: Arc<RenderImage>,
        rendered_zoom: f32,
    },
    Font {
        family: String,
        glyphs: u16,
    },
    /// The extension is in the family but has no decoder yet.
    Unsupported,
    /// The file did not decode; the message says why.
    Failed(String),
}

pub struct ImageView {
    focus_handle: FocusHandle,
    path: PathBuf,
    name: String,
    file_len: u64,
    shown: Shown,
    /// The chosen zoom, or none to fit the view.
    zoom: Option<f32>,
    /// The size of the picture area as last laid out; the fit zoom follows it.
    viewport: Option<Size<Pixels>>,
    scroll: ScrollHandle,
    message: Option<String>,
}

impl ImageView {
    /// Decode the file at `path` for its kind.
    pub fn new(path: &Path, bytes: &[u8], _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        // The extension names the kind; a file the detector routed here by its bytes alone
        // (no extension, or one nobody knows) is decoded by those bytes.
        let kind = images::kind_of(&name).or_else(|| {
            let listed_unsupported = images::UNSUPPORTED.contains(&extension.to_ascii_lowercase().as_str());
            if listed_unsupported {
                None
            } else if images::decode(bytes, extension).is_ok() {
                Some(Kind::Raster)
            } else if images::svg_bytes(bytes).and_then(|svg| images::svg_tree(&svg)).is_ok() {
                Some(Kind::Vector)
            } else if images::font_facts(bytes).is_ok() {
                Some(Kind::Font)
            } else {
                None
            }
        });
        let shown = match kind {
            Some(Kind::Raster) => match images::decode(bytes, extension) {
                Ok(picture) => {
                    let image = render_image(&picture, 0);
                    Shown::Raster { picture, image, rotation: 0 }
                }
                Err(error) => Shown::Failed(error),
            },
            Some(Kind::Vector) => match images::svg_bytes(bytes).and_then(|svg| images::svg_tree(&svg)) {
                Ok(tree) => {
                    let size = (tree.size().width().round() as u32, tree.size().height().round() as u32);
                    match images::rasterize_svg(&tree, 1.0) {
                        Ok(picture) => Shown::Vector { image: render_image(&picture, 0), tree, size, rendered_zoom: 1.0 },
                        Err(error) => Shown::Failed(error),
                    }
                }
                Err(error) => Shown::Failed(error),
            },
            Some(Kind::Font) => match images::font_facts(bytes) {
                Ok((family, glyphs)) => {
                    match cx.text_system().add_fonts(vec![Cow::Owned(bytes.to_vec())]) {
                        Ok(()) => Shown::Font { family, glyphs },
                        Err(error) => Shown::Failed(format!("the font could not be loaded: {error}")),
                    }
                }
                Err(error) => Shown::Failed(error),
            },
            None => Shown::Unsupported,
        };
        Self {
            focus_handle: cx.focus_handle(),
            path: path.to_path_buf(),
            name,
            file_len: bytes.len() as u64,
            shown,
            zoom: None,
            viewport: None,
            scroll: ScrollHandle::new(),
            message: None,
        }
    }

    pub fn shown(&self) -> &Shown {
        &self.shown
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The decoded picture, for a raster file.
    pub fn picture(&self) -> Option<&Picture> {
        match &self.shown {
            Shown::Raster { picture, .. } => Some(picture),
            _ => None,
        }
    }

    /// The size the image is drawn at before zoom: the picture turned by its rotation,
    /// the vector's rendered size, none for a font or a failure.
    pub fn drawn_size(&self) -> Option<(u32, u32)> {
        match &self.shown {
            Shown::Raster { picture, rotation, .. } => Some(if rotation % 2 == 1 {
                (picture.height, picture.width)
            } else {
                (picture.width, picture.height)
            }),
            Shown::Vector { size, .. } => Some(*size),
            _ => None,
        }
    }

    /// Rasterize the vectors again when the zoom in effect moved away from the zoom they
    /// were rendered at, so they stay sharp; never below their own size.
    fn refresh_vector(&mut self) {
        let wanted = self.effective_zoom().max(1.0);
        if let Shown::Vector { tree, image, rendered_zoom, .. } = &mut self.shown
            && (wanted - *rendered_zoom).abs() > 0.01
            && let Ok(picture) = images::rasterize_svg(tree, wanted)
        {
            *image = render_image(&picture, 0);
            *rendered_zoom = wanted;
        }
    }

    /// The chosen zoom, or none while fitting.
    pub fn zoom(&self) -> Option<f32> {
        self.zoom
    }

    /// The zoom in effect: the choice, or the fit (never above 100 %) once the view is
    /// laid out, or 100 % before that.
    pub fn effective_zoom(&self) -> f32 {
        if let Some(zoom) = self.zoom {
            return zoom;
        }
        match (self.drawn_size(), self.viewport) {
            (Some((w, h)), Some(viewport)) if w > 0 && h > 0 => {
                let fit = (f32::from(viewport.width) / w as f32).min(f32::from(viewport.height) / h as f32);
                fit.min(1.0).max(0.01)
            }
            _ => 1.0,
        }
    }

    pub fn rotation(&self) -> u8 {
        match &self.shown {
            Shown::Raster { rotation, .. } => *rotation,
            _ => 0,
        }
    }

    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// The picture area as last laid out.
    pub fn viewport(&self) -> Option<Size<Pixels>> {
        self.viewport
    }

    pub fn zoom_in(&mut self, cx: &mut Context<Self>) {
        self.zoom = Some((self.effective_zoom() * ZOOM_STEP).min(64.));
        cx.notify();
    }

    pub fn zoom_out(&mut self, cx: &mut Context<Self>) {
        self.zoom = Some((self.effective_zoom() / ZOOM_STEP).max(0.01));
        cx.notify();
    }

    pub fn zoom_actual(&mut self, cx: &mut Context<Self>) {
        self.zoom = Some(1.0);
        cx.notify();
    }

    pub fn zoom_fit(&mut self, cx: &mut Context<Self>) {
        self.zoom = None;
        cx.notify();
    }

    /// Turn a raster image a quarter clockwise.
    pub fn rotate_quarter(&mut self, cx: &mut Context<Self>) {
        if let Shown::Raster { picture, image, rotation } = &mut self.shown {
            *rotation = (*rotation + 1) % 4;
            *image = render_image(picture, *rotation);
            cx.notify();
        }
    }

    /// Write the first frame to `path` in the format its extension names (C-009 write).
    pub fn save_as(&mut self, path: &Path, cx: &mut Context<Self>) -> Result<usize, String> {
        let result = match &self.shown {
            Shown::Raster { picture, .. } => images::save_as(picture, path),
            _ => Err("only a decoded raster image can be saved as another format".to_string()),
        };
        self.message = Some(match &result {
            Ok(written) => format!("saved {} ({written} bytes)", path.display()),
            Err(error) => error.clone(),
        });
        cx.notify();
        result
    }

    fn on_zoom_in(&mut self, _: &ZoomIn, _: &mut Window, cx: &mut Context<Self>) {
        self.zoom_in(cx);
    }

    fn on_zoom_out(&mut self, _: &ZoomOut, _: &mut Window, cx: &mut Context<Self>) {
        self.zoom_out(cx);
    }

    fn on_zoom_actual(&mut self, _: &ZoomActual, _: &mut Window, cx: &mut Context<Self>) {
        self.zoom_actual(cx);
    }

    fn on_zoom_fit(&mut self, _: &ZoomFit, _: &mut Window, cx: &mut Context<Self>) {
        self.zoom_fit(cx);
    }

    fn on_rotate(&mut self, _: &RotateQuarter, _: &mut Window, cx: &mut Context<Self>) {
        self.rotate_quarter(cx);
    }

    /// The facts the side column lists, as `(element id, text)`; the EXIF rows are
    /// `("image-exif", index)`.
    pub fn facts(&self) -> Vec<(ElementId, String)> {
        let mut facts: Vec<(ElementId, String)> = vec![("image-bytes".into(), format!("{} bytes", self.file_len))];
        match &self.shown {
            Shown::Raster { picture, rotation, .. } => {
                facts.insert(0, ("image-size".into(), format!("{} × {} pixels", picture.width, picture.height)));
                facts.push(("image-colour".into(), picture.colour.clone()));
                facts.push((
                    "image-frames".into(),
                    if picture.is_animated() { format!("{} frames", picture.frames.len()) } else { "still".to_string() },
                ));
                facts.push(("image-zoom".into(), zoom_label(self.zoom, self.effective_zoom())));
                facts.push((
                    "image-rotation".into(),
                    if *rotation == 0 {
                        "0°".to_string()
                    } else {
                        let (w, h) = self.drawn_size().unwrap_or((picture.width, picture.height));
                        format!("{}°, drawn {w} × {h}", u16::from(*rotation) * 90)
                    },
                ));
                for (i, (name, value)) in picture.exif.iter().enumerate() {
                    facts.push((("image-exif", i).into(), format!("{name}: {value}")));
                }
            }
            Shown::Vector { size, .. } => {
                facts.insert(0, ("image-size".into(), format!("{} × {} pixels, from vectors", size.0, size.1)));
                facts.push(("image-zoom".into(), zoom_label(self.zoom, self.effective_zoom())));
            }
            Shown::Font { family, glyphs } => {
                facts.push(("image-font-family".into(), family.clone()));
                facts.push(("image-font-glyphs".into(), format!("{glyphs} glyphs")));
            }
            Shown::Unsupported => {
                facts.push(("image-note".into(), format!("No decoder for .{} yet; Ctrl+Shift+B shows the bytes.", extension_of(&self.name))));
            }
            Shown::Failed(error) => {
                facts.push(("image-note".into(), format!("Cannot decode this file: {error}")));
            }
        }
        facts
    }

    /// The delays, in milliseconds, of the frames handed to the window, in order: what the
    /// image element plays.
    pub fn rendered_frame_delays(&self) -> Vec<u32> {
        match &self.shown {
            Shown::Raster { image, .. } | Shown::Vector { image, .. } => (0..image.frame_count())
                .map(|i| {
                    let (numer, denom) = image.delay(i).numer_denom_ms();
                    if denom == 0 { 0 } else { numer / denom }
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// How far the picture area is scrolled (negative when panned right or down).
    pub fn scroll_offset(&self) -> gpui_kit::Point<Pixels> {
        self.scroll.offset()
    }

    /// The size, in pixels, of the first frame handed to the window: turned for a raster,
    /// rasterized at the zoom in effect for vectors.
    pub fn rendered_size(&self) -> Option<(u32, u32)> {
        match &self.shown {
            Shown::Raster { image, .. } | Shown::Vector { image, .. } => {
                let s = image.size(0);
                Some((s.width.0 as u32, s.height.0 as u32))
            }
            _ => None,
        }
    }

    /// One pixel of the first frame handed to the window, as the window holds it: BGRA.
    pub fn rendered_pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        let (width, height) = self.rendered_size()?;
        if x >= width || y >= height {
            return None;
        }
        let image = match &self.shown {
            Shown::Raster { image, .. } | Shown::Vector { image, .. } => image,
            _ => return None,
        };
        let bytes = image.as_bytes(0)?;
        let at = ((y * width + x) * 4) as usize;
        bytes.get(at..at + 4).map(|p| [p[0], p[1], p[2], p[3]])
    }

    /// The zoom the vectors were last rasterized at.
    pub fn rendered_zoom(&self) -> Option<f32> {
        match &self.shown {
            Shown::Vector { rendered_zoom, .. } => Some(*rendered_zoom),
            _ => None,
        }
    }
}

fn extension_of(name: &str) -> String {
    name.rsplit('.').next().unwrap_or("").to_ascii_lowercase()
}

/// What the zoom fact says.
pub fn zoom_label(chosen: Option<f32>, effective: f32) -> String {
    let percent = (effective * 100.).round() as u32;
    match chosen {
        Some(_) => format!("{percent} %"),
        None => format!("fit, {percent} %"),
    }
}

/// The picture's frames as the window paints them (BGRA), turned by `rotation` quarters.
pub fn render_image(picture: &Picture, rotation: u8) -> Arc<RenderImage> {
    let frames: Vec<image::Frame> = picture
        .frames
        .iter()
        .map(|frame| {
            let mut buffer = RgbaImage::from_raw(picture.width, picture.height, frame.rgba.clone())
                .expect("a frame holds width × height RGBA pixels");
            buffer = match rotation % 4 {
                1 => image::imageops::rotate90(&buffer),
                2 => image::imageops::rotate180(&buffer),
                3 => image::imageops::rotate270(&buffer),
                _ => buffer,
            };
            for pixel in buffer.pixels_mut() {
                pixel.0.swap(0, 2);
            }
            image::Frame::from_parts(buffer, 0, 0, Delay::from_numer_denom_ms(frame.delay_ms.max(1), 1))
        })
        .collect();
    Arc::new(RenderImage::new(frames))
}

impl Focusable for ImageView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

fn fact_row(id: impl Into<ElementId>, text: String) -> impl IntoElement {
    div()
        .id(id.into())
        .role(Role::Status)
        .test_support()
        .aria_label(text.clone())
        .text_sm()
        .child(text)
}

impl Render for ImageView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.refresh_vector();
        let theme = cx.theme();
        let (light, dark) = if theme.is_dark() {
            (theme.muted, theme.background)
        } else {
            (theme.background, theme.muted)
        };
        let muted_fg = theme.muted_foreground;
        let border = theme.border;
        let this = cx.entity();
        let facts = self.facts();
        let message = self.message.clone();
        let zoom = self.effective_zoom();
        let drawn = self.drawn_size();
        let image = match &self.shown {
            Shown::Raster { image, .. } | Shown::Vector { image, .. } => Some(image.clone()),
            _ => None,
        };
        let specimen = match &self.shown {
            Shown::Font { family, .. } => Some(family.clone()),
            _ => None,
        };
        let fitting = self.zoom.is_none();

        // The picture area measures itself; the fit zoom follows its size.
        let measure = canvas(
            move |bounds: Bounds<Pixels>, _window, cx| {
                this.update(cx, |view, cx| {
                    if view.viewport != Some(bounds.size) {
                        view.viewport = Some(bounds.size);
                        cx.notify();
                    }
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();

        let picture = match (image, drawn) {
            (Some(image), Some((w, h))) => {
                let (bw, bh) = (px(w as f32 * zoom), px(h as f32 * zoom));
                let checker = canvas(
                    |_, _, _| {},
                    move |bounds: Bounds<Pixels>, _, window, _| {
                        let visible = window.content_mask().bounds.intersect(&bounds);
                        let step = px(CHECKER);
                        let mut y = bounds.origin.y;
                        let mut row = 0usize;
                        while y < visible.bottom() {
                            if y + step > visible.top() {
                                let mut x = bounds.origin.x;
                                let mut col = 0usize;
                                while x < visible.right() {
                                    if x + step > visible.left() {
                                        let colour = if (row + col) % 2 == 0 { light } else { dark };
                                        let quad = Bounds::new(gpui_kit::point(x, y), size(step, step)).intersect(&bounds);
                                        window.paint_quad(gpui_kit::fill(quad, colour));
                                    }
                                    x += step;
                                    col += 1;
                                }
                            }
                            y += step;
                            row += 1;
                        }
                    },
                )
                .absolute()
                .size_full();
                Some(
                    div()
                        .id("image-box")
                        .test_support()
                        .relative()
                        .flex_shrink_0()
                        .w(bw)
                        .h(bh)
                        .child(checker)
                        .child(img(image).size_full().object_fit(ObjectFit::Fill)),
                )
            }
            _ => None,
        };

        v_flex()
            .id("image-view")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_zoom_in))
            .on_action(cx.listener(Self::on_zoom_out))
            .on_action(cx.listener(Self::on_zoom_actual))
            .on_action(cx.listener(Self::on_zoom_fit))
            .on_action(cx.listener(Self::on_rotate))
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                h_flex()
                    .size_full()
                    .child(
                        div()
                            .id("image-scroll")
                            .test_support()
                            .relative()
                            .flex_1()
                            .h_full()
                            .min_w_0()
                            .overflow_scroll()
                            .track_scroll(&self.scroll)
                            .flex()
                            .when(fitting, |area| area.items_center().justify_center())
                            .child(measure)
                            .children(picture)
                            .children(specimen.map(|family| {
                                v_flex()
                                    .id("image-specimen")
                                    .test_support()
                                    .aria_label(family.clone())
                                    .p_6()
                                    .gap_3()
                                    .font_family(family.clone())
                                    .child(div().text_size(px(36.)).child("Sfoglia leafs through fonts too"))
                                    .child(div().text_size(px(24.)).child("The quick brown fox jumps over the lazy dog"))
                                    .child(div().text_size(px(16.)).child("ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz"))
                                    .child(div().text_size(px(16.)).child("0123456789 !?&@#%(){}[]<>+-=/*"))
                                    .child(div().text_size(px(12.)).text_color(muted_fg).child(family))
                            })),
                    )
                    .child(
                        v_flex()
                            .id("image-facts")
                            .test_support()
                            .flex_shrink_0()
                            .w(px(260.))
                            .h_full()
                            .gap_1()
                            .p_3()
                            .border_l_1()
                            .border_color(border)
                            .text_color(muted_fg)
                            .child(div().text_sm().font_weight(gpui_kit::FontWeight::SEMIBOLD).child(SharedString::from(self.name.clone())))
                            .children(facts.into_iter().map(|(id, text)| fact_row(id, text)))
                            .children(message.map(|m| fact_row("image-message", m))),
                    ),
            )
    }
}
