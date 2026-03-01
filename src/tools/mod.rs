//! Shared tool definitions and executor for Scriba.
//!
//! This module provides the canonical set of 14 tools used by both the
//! in-app agent chat and the MCP server. Each consumer wraps these into
//! its own wire format (Anthropic tool_use vs MCP JSON-RPC).

pub mod definitions;
pub mod executor;

pub use definitions::{all_tool_schemas, ToolSchema};
pub use executor::{execute_tool, ToolResult};
