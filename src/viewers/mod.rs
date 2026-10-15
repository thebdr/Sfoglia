//! The viewers a file can open in. Each is named by the contract clause it implements.

pub mod rules_panel;
pub mod text_view;

pub use rules_panel::{RulesPanel, RulesStore};
pub use text_view::TextView;
