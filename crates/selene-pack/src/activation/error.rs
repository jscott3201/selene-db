//! Activation state-machine errors.

use crate::{Gate, ManifestError};

/// Failure while advancing a procedure-pack lifecycle state.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ActivationError {
    /// Manifest parsing or validation failed.
    #[error("manifest activation validation failed: {source}")]
    Manifest {
        /// Underlying manifest error.
        #[source]
        source: ManifestError,
    },
    /// A pack with the same name already occupies the activation registry.
    #[error("procedure pack {pack_name} is already active or deprecated")]
    PackAlreadyActive {
        /// Conflicting pack name.
        pack_name: String,
    },
    /// Lifecycle sink refused to record an event.
    #[error("procedure pack lifecycle sink refused event: {detail}")]
    SinkRefused {
        /// Stable failure detail.
        detail: String,
    },
}

impl ActivationError {
    /// Return the validation gate that owns this error, when any.
    #[must_use]
    pub fn gate(&self) -> Option<Gate> {
        match self {
            Self::Manifest { source } => Some(source.gate()),
            Self::PackAlreadyActive { .. } => Some(Gate::RegistryConflictDetection),
            Self::SinkRefused { .. } => None,
        }
    }
}

impl From<ManifestError> for ActivationError {
    fn from(source: ManifestError) -> Self {
        Self::Manifest { source }
    }
}
