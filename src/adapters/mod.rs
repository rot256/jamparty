//! I/O at the edges: iroh endpoints, control-line framing, HTTP bodies, the
//! joiner's settings file, the runtime socket and the command target.

pub(crate) mod command;
pub(crate) mod endpoint;
pub(crate) mod http;
pub(crate) mod lines;
pub(crate) mod output;
pub(crate) mod path;
pub(crate) mod runtime;
pub(crate) mod settings;
