//! The learning machinery (`docs/spec/brain.md`, `docs/spec/adaptivelearning.md`).
//!
//! Everything here is pure: no serenity, no poise, no I/O. Time and randomness
//! are injected so every test is deterministic.

pub mod addenda;
pub mod ask_signature;
pub mod budget;
pub mod correction;
pub mod dqn;
pub mod intent;
pub mod nn;
pub mod outcome;
pub mod registry;
pub mod replay;
pub mod reward;
pub mod social;
pub mod state;
pub mod style_signal;
pub mod telemetry;

pub mod erasure;

pub mod quality_evaluation;
