//! Sfoglia entry point: open the main window (C-001) with every path given on the
//! command line already open (C-004).

use std::path::PathBuf;

use gpui_kit::AppContext as _;
use sfoglia::{Workspace, window_options};

fn main() {
    let paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            sfoglia::init(cx);
            gpui_kit::open_window(window_options(), cx, |window, cx| {
                cx.new(|cx| {
                    let mut workspace = Workspace::new(window, cx);
                    workspace.open_paths(&paths, window, cx);
                    workspace
                })
            })
            .expect("failed to open the main window");
        });
}
