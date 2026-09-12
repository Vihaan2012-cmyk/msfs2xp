//! Reading Microsoft Flight Simulator BGL scenery files.

pub mod codec;
pub mod file;
pub mod guid;
pub mod inspect;
pub mod modellib;
pub mod reader;
pub mod records;

#[cfg(test)]
pub mod testutil;

pub use file::{classify, BglFile, FileClass, RecordSlice};
pub use reader::{BglError, Reader};
pub use records::raw::Variant;
