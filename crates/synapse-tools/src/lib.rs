//! synapse-tools: built-in tools and permission engine (see docs/ARCHITECTURE.md).

pub mod permissions;

pub use permissions::{Decision, Effect, Engine, Request, Rule, RuleScope};
