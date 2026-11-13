//! Pure decisions and models: no I/O, and no clock or randomness of its own
//! (both are injected through [`clock::Clock`] and [`rng::Rng`]).

pub(crate) mod clock;
pub(crate) mod ids;
pub(crate) mod invalid;
pub(crate) mod name;
pub(crate) mod port;
pub(crate) mod queue;
pub(crate) mod ratelimit;
pub(crate) mod request;
pub(crate) mod rng;
pub(crate) mod secs;
pub(crate) mod settings;
pub(crate) mod text;
pub(crate) mod token;
pub(crate) mod version;
