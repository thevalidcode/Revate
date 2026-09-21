//! Project file management.
//! Handles .revate project file format, I/O, and migration.
//! Validates project files on load and provides error reporting.

pub mod format;
pub mod io;
pub mod migrate;
