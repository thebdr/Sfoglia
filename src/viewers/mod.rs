//! The viewers a file can open in. Each is named by the contract clause it implements.

pub mod hex_view;
pub mod rules_panel;
pub mod text_view;

pub use hex_view::HexView;
pub use rules_panel::{RulesPanel, RulesStore};
pub use text_view::TextView;
