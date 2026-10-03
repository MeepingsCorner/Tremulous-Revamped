//! Kernel `rng` — the deterministic `rand()` shared by the port and the C
//! oracle (dec013). The oracle's shims.c defines the identical generator, so
//! every `rand()` / `random()` call site in the original consumes the same
//! stream in both programs.

/// glibc RAND_MAX, which the original's arithmetic (`num /= RAND_MAX / 6 + 1`) assumes.
pub const RAND_MAX: i32 = 2147483647;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rng {
    pub state: u32,
}

impl Rng {
    pub const fn new(seed: u32) -> Self {
        Rng { state: seed }
    }
    /// `rand()`: 31-bit output of a 32-bit LCG (Numerical Recipes constants).
    #[inline]
    pub fn rand(&mut self) -> i32 {
        self.state = self.state.wrapping_mul(1103515245).wrapping_add(12345);
        ((self.state >> 1) & 0x7fff_ffff) as i32
    }
    /// `random()` macro: ((rand() & 0x7fff) / ((float)0x7fff)).
    #[inline]
    pub fn random(&mut self) -> f32 {
        (self.rand() & 0x7fff) as f32 / 0x7fff as f32
    }
    /// `crandom()` macro: (2.0 * (random() - 0.5)), a double expression.
    #[inline]
    pub fn crandom(&mut self) -> f64 {
        2.0 * (self.random() as f64 - 0.5)
    }
}

impl Default for Rng {
    fn default() -> Self {
        Rng::new(0x5eed_1234)
    }
}
