//! Nyx Voice's wire types.
//!
//! - [`asr`]: frames between the engine and its speech recognizer worker.
//! - [`voice`]: the `nyx-voiced` socket protocol apps use.

pub mod asr;
pub mod voice;
