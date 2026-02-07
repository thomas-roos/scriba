//! Entity management module.
//!
//! This module provides entity registry and smart linking capabilities
//! for correlating mentions across recordings.

mod linker;
mod registry;

pub use linker::EntityLinker;
pub use registry::EntityRegistry;
