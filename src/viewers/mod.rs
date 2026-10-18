//! The viewers a file can open in. Each is named by the contract clause it implements.

pub mod archive_view;
pub mod data_view;
pub mod hex_view;
pub mod image_view;
pub mod markdown_view;
pub mod pdf_view;
pub mod rules_panel;
pub mod sheet_view;
pub mod text_view;

pub use archive_view::ArchiveView;
pub use data_view::DataView;
pub use hex_view::HexView;
pub use image_view::ImageView;
pub use markdown_view::MarkdownView;
pub use pdf_view::PdfView;
pub use rules_panel::{RulesPanel, RulesStore};
pub use sheet_view::SheetView;
pub use text_view::TextView;
