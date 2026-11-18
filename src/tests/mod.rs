//! In-process integration tests: shares and joiners over iroh in one
//! process, direct addresses, no relays.
// The page socket's messages are read as JSON values, whose index gives
// null for what is missing rather than panicking.
#![allow(clippy::indexing_slicing)]

mod admission;
mod admission_hello;
mod agent;
mod amplification;
mod attachments;
mod delivery;
mod fake_share;
mod harness;
mod hostile_share;
mod local_rules;
mod logs;
mod mirror;
mod presence;
mod proxy_limits;
mod requests;
mod session;
mod settings;
mod upstream;
mod versions;
