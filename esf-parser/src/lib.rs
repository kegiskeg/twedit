//! `esf-parser` is a library for parsing and modifying Total War ESF (Empire Save Format) files.
//! It supports the ABCD and ABCE formats, which are used to store game save state and campaign data.
//!
//! This crate provides:
//! - `parser`: Functions to load and parse ESF files from bytes or disk.
//! - `objects`: Structural components like nodes, values, and the document arena.
//! - `enums`: Constants for magic headers and type tags.

pub mod enums;
pub mod objects;
pub mod parser;
pub mod pack_parser;

pub use enums::*;
pub use objects::*;
pub use parser::*;
