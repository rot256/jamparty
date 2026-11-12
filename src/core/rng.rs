//! Injected randomness.

#[cfg(test)]
use std::sync::{Mutex, PoisonError};

#[cfg(test)]
use sha3::{Digest, Sha3_256};

/// A source of random bytes. Production uses [`SystemRng`]; tests use
/// [`SeededRng`].
pub(crate) trait Rng: Send + Sync {
    fn fill(&self, dest: &mut [u8]);
}

/// Returns `N` random bytes from `rng`.
pub(crate) fn random_array<const N: usize>(rng: &dyn Rng) -> [u8; N] {
    let mut bytes = [0; N];
    rng.fill(&mut bytes);
    bytes
}

/// The operating system's generator, through `getrandom`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SystemRng;

impl Rng for SystemRng {
    /// # Panics
    ///
    /// Panics if the operating system cannot provide random bytes: nothing
    /// that needs a seed, token or id can continue safely without them.
    fn fill(&self, dest: &mut [u8]) {
        if let Err(error) = getrandom::fill(dest) {
            panic!("operating system random number generator failed: {error}");
        }
    }
}

/// A deterministic generator for tests: SHA3-256 over (seed, counter).
/// Not for production use.
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct SeededRng {
    seed: u64,
    counter: Mutex<u64>,
}

#[cfg(test)]
impl SeededRng {
    #[must_use]
    pub(crate) fn new(seed: u64) -> Self {
        Self {
            seed,
            counter: Mutex::new(0),
        }
    }
}

#[cfg(test)]
impl Rng for SeededRng {
    fn fill(&self, dest: &mut [u8]) {
        let mut counter = self.counter.lock().unwrap_or_else(PoisonError::into_inner);
        for chunk in dest.chunks_mut(32) {
            let block = Sha3_256::new()
                .chain_update(self.seed.to_le_bytes())
                .chain_update(counter.to_le_bytes())
                .finalize();
            *counter = counter.wrapping_add(1);
            for (out, byte) in chunk.iter_mut().zip(block.iter()) {
                *out = *byte;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::core::rng::{SeededRng, SystemRng, random_array};

    #[test]
    fn seeded_rng_is_deterministic_and_advances() {
        let first: [u8; 40] = random_array(&SeededRng::new(7));
        let again: [u8; 40] = random_array(&SeededRng::new(7));
        assert_eq!(first, again);

        let rng = SeededRng::new(7);
        let a: [u8; 16] = random_array(&rng);
        let b: [u8; 16] = random_array(&rng);
        assert_ne!(a, b);

        let other: [u8; 40] = random_array(&SeededRng::new(8));
        assert_ne!(first, other);
    }

    #[test]
    fn system_rng_fills() {
        let a: [u8; 32] = random_array(&SystemRng);
        let b: [u8; 32] = random_array(&SystemRng);
        assert_ne!(a, b);
    }
}
