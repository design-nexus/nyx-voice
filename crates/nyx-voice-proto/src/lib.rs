//! Nyx Voice's wire types.
//!
//! - [`asr`]: frames between the engine and its speech recognizer worker.
//! - [`voice`]: the `nyx-voiced` socket protocol apps use.
//! - [`settings`]: the speech settings the engine takes, as config files hold them.

pub mod asr;
pub mod settings;
pub mod voice;
