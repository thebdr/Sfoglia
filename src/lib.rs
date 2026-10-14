//! Sfoglia: a viewer for any file, with editing where it is worth it.
//!
//! The behaviour of this crate is governed by the contract in `.zen/contract.md`;
//! each module names the clause it implements.

pub mod detect;
pub mod generated;
pub mod shell;
pub mod text;
pub mod viewers;
pub mod workspace;

pub use generated::families::Family;
pub use shell::{APP_NAME, window_options};
pub use workspace::{OpenFiles, ToggleTheme, Workspace, init};
