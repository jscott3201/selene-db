//! In-memory procedure-pack activation state machine.

mod content_hash;
mod error;
mod events;
mod principal;
mod registry;
mod states;

pub use content_hash::ContentHash;
pub use error::ActivationError;
pub use events::{LifecycleEvent, LifecycleSink, NoopSink};
pub use principal::Principal;
pub use registry::{ActivationEntry, ActivationRegistry, ActivationStatus};
pub use states::{Active, Deprecated, Disabled, Staged, Uploaded, Validating};
