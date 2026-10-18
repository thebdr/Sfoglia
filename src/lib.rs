//! Sfoglia: a viewer for any file, with editing where it is worth it.
//!
//! The behaviour of this crate is governed by the contract in `.zen/contract.md`;
//! each module names the clause it implements.

pub mod data;
pub mod detect;
pub mod generated;
pub mod hex;
pub mod images;
pub mod markdown;
pub mod net;
#[cfg(windows)]
pub mod pdf_host;
pub mod pdf_pages;
pub mod rules;
pub mod sheet;
pub mod shell;
pub mod text;
pub mod tools;
pub mod viewers;
pub mod workspace;

pub use generated::families::Family;
pub use shell::{APP_NAME, window_options};
pub use workspace::{OpenFiles, SaveFile, SaveImageAs, ToggleHexView, ToggleHighlightRules, ToggleTheme, Workspace, init};
