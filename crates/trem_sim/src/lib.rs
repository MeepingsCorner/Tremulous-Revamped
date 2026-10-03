//! trem_sim: Tremulous 1.2 gameplay simulation (engine-agnostic).
//!
//! `gen`    — projected from sheets/ by build.rs (never hand-edited, D6)
//! `kernel` — hand-written algorithms, each listed in sheets/kernel.tsv (D4)

#![allow(clippy::too_many_arguments, clippy::needless_range_loop)]

pub mod gen {
    include!(concat!(env!("OUT_DIR"), "/gen_mods.rs"));
}
pub mod kernel;
pub mod cmq;
pub mod parity;

pub use kernel::*;
