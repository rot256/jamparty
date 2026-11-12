//! Pure decisions and models: no I/O, and no clock or randomness of its own
//! (both are injected through [`clock::Clock`] and [`rng::Rng`]).

pub(crate) mod access;
pub(crate) mod allowlist;
pub(crate) mod clock;
pub(crate) mod cookie;
pub(crate) mod entry;
pub(crate) mod host;
pub(crate) mod ids;
pub(crate) mod pairing;
pub(crate) mod queue;
pub(crate) mod ratelimit;
pub(crate) mod rng;
pub(crate) mod safe_path;
pub(crate) mod session;
pub(crate) mod store;
pub(crate) mod text;
pub(crate) mod version;
