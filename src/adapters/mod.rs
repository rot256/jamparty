//! I/O at the edges: iroh endpoints, control-line framing, HTTP bodies, the
//! joiner's settings file, the runtime socket, the command target
//! and the t3 target (an MCP client).

pub(crate) mod command;
pub(crate) mod endpoint;
pub(crate) mod http;
pub(crate) mod lines;
pub(crate) mod mcp;
pub(crate) mod output;
pub(crate) mod runtime;
pub(crate) mod settings;
pub(crate) mod t3;
