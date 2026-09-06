//! Pure Daymark rules. No clock, storage, network or authentication dependencies.
//!
//! Callers supply a trusted authenticated user ID and UTC audit time. This crate
//! scopes personal operations by that ID; authenticating it is an adapter concern.
//! Calendar mutation is a privileged application operation, not an admin bypass
//! for personal records. The in-memory model is a reference transaction boundary;
//! a persistence adapter must provide equivalent isolation and atomicity.

mod ledger;
mod model;
#[cfg(feature = "serde")]
mod serialization;

pub use ledger::*;
pub use model::*;
