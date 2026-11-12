//! I/O at the edges: iroh endpoints, control-line framing, the joiner's key
//! file, the runtime socket, the command target and the t3 target (an MCP
//! client).

pub(crate) mod command;
pub(crate) mod endpoint;
pub(crate) mod keyfile;
pub(crate) mod lines;
pub(crate) mod mcp;
pub(crate) mod runtime;
pub(crate) mod t3;
