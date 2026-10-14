//! Sfoglia entry point: open the application shell (C-001).

use gpui_kit::AppContext as _;
use sfoglia::{Shell, window_options};

fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(window_options(), cx, |_, cx| cx.new(|_| Shell))
                .expect("failed to open the main window");
        });
}
