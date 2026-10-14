//! Sfoglia: a viewer for any file, with editing where it is worth it.
//!
//! The behaviour of this crate is governed by the contract in `.zen/contract.md`;
//! each module names the clause it implements.

pub mod shell;

pub use shell::{APP_NAME, Shell, window_options};
