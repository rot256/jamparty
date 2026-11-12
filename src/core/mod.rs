//! Pure decisions and models: no I/O, and no clock or randomness of its own
//! (both are injected through [`clock::Clock`] and [`rng::Rng`]).

pub(crate) mod clock;
pub(crate) mod ids;
pub(crate) mod name;
// The delivery state machine is driven from implementation step 6 on.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod queue;
pub(crate) mod ratelimit;
pub(crate) mod request;
pub(crate) mod rng;
pub(crate) mod text;
pub(crate) mod token;
pub(crate) mod version;
