//! Claude integration for fushion.
//!
//! - [`session`]: shared build state on disk.
//! - [`claude_api`]: the Messages API tool loop (bring your own API key).
//! - [`claude_code`]: drive the user's Claude Code CLI (their subscription)
//!   with fushion's MCP server attached.

pub mod claude_api;
pub mod claude_code;
pub mod custom;
pub mod session;

pub use session::{BuildOutput, ExportJob, Session};
