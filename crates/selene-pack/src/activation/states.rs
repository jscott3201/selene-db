//! Typestate structs for procedure-pack activation.

use jiff::Timestamp;
use selene_core::IStr;
use serde_json::Value;

use super::{
    ActivationEntry, ActivationError, ActivationRegistry, ContentHash, LifecycleEvent,
    LifecycleSink, Principal,
};
use crate::{ProcedurePackManifest, parse_manifest};

/// Uploaded procedure-pack bytes awaiting validation.
#[derive(Clone, Debug)]
pub struct Uploaded {
    raw_bytes: Vec<u8>,
    principal: Principal,
    uploaded_at: Timestamp,
}

impl Uploaded {
    /// Construct an uploaded procedure-pack payload.
    #[must_use]
    pub fn new(
        raw_bytes: impl Into<Vec<u8>>,
        principal: Principal,
        uploaded_at: Timestamp,
    ) -> Self {
        Self {
            raw_bytes: raw_bytes.into(),
            principal,
            uploaded_at,
        }
    }

    /// Return the raw manifest bytes.
    #[must_use]
    pub fn raw_bytes(&self) -> &[u8] {
        &self.raw_bytes
    }

    /// Return the principal that uploaded the pack.
    #[must_use]
    pub const fn principal(&self) -> Principal {
        self.principal
    }

    /// Return the upload timestamp.
    #[must_use]
    pub const fn uploaded_at(&self) -> Timestamp {
        self.uploaded_at
    }

    /// Parse and validate the uploaded manifest.
    ///
    /// # Errors
    ///
    /// Returns [`ActivationError::Manifest`] when manifest validation fails, or
    /// [`ActivationError::SinkRefused`] when the sink rejects the validation
    /// failure event.
    pub fn validate(
        self,
        sink: &dyn LifecycleSink,
        now: Timestamp,
    ) -> Result<Validating, ActivationError> {
        match parse_manifest(&self.raw_bytes) {
            Ok(manifest) => Ok(Validating {
                manifest,
                raw_bytes: self.raw_bytes,
                principal: self.principal,
                uploaded_at: self.uploaded_at,
                validating_at: now,
            }),
            Err(source) => {
                let pack_name = extract_pack_name(&self.raw_bytes);
                let error = source.to_string();
                let event = LifecycleEvent::ValidationFailed {
                    pack_name,
                    principal: self.principal,
                    error,
                    at: now,
                };
                sink.record(&event)?;
                Err(ActivationError::Manifest { source })
            }
        }
    }
}

/// Parsed and validated pack ready to be staged.
#[derive(Clone, Debug)]
pub struct Validating {
    manifest: ProcedurePackManifest,
    raw_bytes: Vec<u8>,
    principal: Principal,
    uploaded_at: Timestamp,
    validating_at: Timestamp,
}

impl Validating {
    /// Return the validated manifest.
    #[must_use]
    pub const fn manifest(&self) -> &ProcedurePackManifest {
        &self.manifest
    }

    /// Return the original manifest bytes.
    #[must_use]
    pub fn raw_bytes(&self) -> &[u8] {
        &self.raw_bytes
    }

    /// Return the activation principal.
    #[must_use]
    pub const fn principal(&self) -> Principal {
        self.principal
    }

    /// Return the upload timestamp.
    #[must_use]
    pub const fn uploaded_at(&self) -> Timestamp {
        self.uploaded_at
    }

    /// Return the validation timestamp.
    #[must_use]
    pub const fn validating_at(&self) -> Timestamp {
        self.validating_at
    }

    /// Stage the validated manifest.
    ///
    /// # Errors
    ///
    /// Returns [`ActivationError::SinkRefused`] when the sink rejects the staged
    /// event.
    pub fn stage(
        self,
        sink: &dyn LifecycleSink,
        now: Timestamp,
    ) -> Result<Staged, ActivationError> {
        let content_hash = ContentHash::from_validated_manifest(&self.manifest);
        let event = LifecycleEvent::Staged {
            pack_name: self.manifest.pack_name.clone(),
            content_hash,
            principal: self.principal,
            at: now,
        };
        sink.record(&event)?;
        Ok(Staged {
            manifest: self.manifest,
            content_hash,
            principal: self.principal,
            staged_at: now,
        })
    }
}

/// Staged pack awaiting activation.
#[derive(Clone, Debug)]
pub struct Staged {
    manifest: ProcedurePackManifest,
    content_hash: ContentHash,
    principal: Principal,
    staged_at: Timestamp,
}

impl Staged {
    /// Return the pack name.
    #[must_use]
    pub fn pack_name(&self) -> &str {
        &self.manifest.pack_name
    }

    /// Return the validated manifest.
    #[must_use]
    pub const fn manifest(&self) -> &ProcedurePackManifest {
        &self.manifest
    }

    /// Return the pack content hash.
    #[must_use]
    pub const fn content_hash(&self) -> ContentHash {
        self.content_hash
    }

    /// Return the activation principal.
    #[must_use]
    pub const fn principal(&self) -> Principal {
        self.principal
    }

    /// Return the staging timestamp.
    #[must_use]
    pub const fn staged_at(&self) -> Timestamp {
        self.staged_at
    }

    /// Activate this staged pack.
    ///
    /// # Errors
    ///
    /// Returns [`ActivationError::PackAlreadyActive`] when the pack name is
    /// already occupied, or [`ActivationError::SinkRefused`] when the sink
    /// rejects the activation event.
    pub fn activate(
        self,
        sink: &dyn LifecycleSink,
        registry: &mut ActivationRegistry,
        now: Timestamp,
    ) -> Result<Active, ActivationError> {
        let pack_name = self.manifest.pack_name.clone();
        if registry.is_occupied(&pack_name) {
            return Err(ActivationError::PackAlreadyActive { pack_name });
        }
        let event = LifecycleEvent::Activated {
            pack_name: pack_name.clone(),
            content_hash: self.content_hash,
            principal: self.principal,
            at: now,
        };
        sink.record(&event)?;

        let active = Active {
            manifest: self.manifest,
            content_hash: self.content_hash,
            principal: self.principal,
            staged_at: self.staged_at,
            activated_at: now,
        };
        registry.insert_active(ActivationEntry::active(
            active.manifest.clone(),
            active.content_hash,
            active.principal,
            active.activated_at,
        ));
        Ok(active)
    }
}

/// Active pack occupying its pack name.
#[derive(Clone, Debug)]
pub struct Active {
    manifest: ProcedurePackManifest,
    content_hash: ContentHash,
    principal: Principal,
    staged_at: Timestamp,
    activated_at: Timestamp,
}

impl Active {
    /// Return the pack name.
    #[must_use]
    pub fn pack_name(&self) -> &str {
        &self.manifest.pack_name
    }

    /// Return the validated manifest.
    #[must_use]
    pub const fn manifest(&self) -> &ProcedurePackManifest {
        &self.manifest
    }

    /// Return the pack content hash.
    #[must_use]
    pub const fn content_hash(&self) -> ContentHash {
        self.content_hash
    }

    /// Return the activation principal.
    #[must_use]
    pub const fn principal(&self) -> Principal {
        self.principal
    }

    /// Return the staging timestamp.
    #[must_use]
    pub const fn staged_at(&self) -> Timestamp {
        self.staged_at
    }

    /// Return the activation timestamp.
    #[must_use]
    pub const fn activated_at(&self) -> Timestamp {
        self.activated_at
    }

    /// Deprecate this active pack while preserving name occupancy.
    ///
    /// # Errors
    ///
    /// Returns [`ActivationError::SinkRefused`] when the sink rejects the
    /// deprecation event.
    pub fn deprecate(
        self,
        sink: &dyn LifecycleSink,
        registry: &mut ActivationRegistry,
        reason: IStr,
        now: Timestamp,
    ) -> Result<Deprecated, ActivationError> {
        let event = LifecycleEvent::Deprecated {
            pack_name: self.manifest.pack_name.clone(),
            reason,
            principal: self.principal,
            at: now,
        };
        sink.record(&event)?;

        registry.mark_deprecated(ActivationEntry::deprecated_from_active(&self, now));
        Ok(Deprecated {
            manifest: self.manifest,
            content_hash: self.content_hash,
            principal: self.principal,
            staged_at: self.staged_at,
            activated_at: self.activated_at,
            deprecated_at: now,
            reason,
        })
    }
}

/// Deprecated pack that still occupies its pack name.
#[derive(Clone, Debug)]
pub struct Deprecated {
    manifest: ProcedurePackManifest,
    content_hash: ContentHash,
    principal: Principal,
    staged_at: Timestamp,
    activated_at: Timestamp,
    deprecated_at: Timestamp,
    reason: IStr,
}

impl Deprecated {
    /// Return the pack name.
    #[must_use]
    pub fn pack_name(&self) -> &str {
        &self.manifest.pack_name
    }

    /// Return the validated manifest.
    #[must_use]
    pub const fn manifest(&self) -> &ProcedurePackManifest {
        &self.manifest
    }

    /// Return the pack content hash.
    #[must_use]
    pub const fn content_hash(&self) -> ContentHash {
        self.content_hash
    }

    /// Return the activation principal.
    #[must_use]
    pub const fn principal(&self) -> Principal {
        self.principal
    }

    /// Return the staging timestamp.
    #[must_use]
    pub const fn staged_at(&self) -> Timestamp {
        self.staged_at
    }

    /// Return the activation timestamp.
    #[must_use]
    pub const fn activated_at(&self) -> Timestamp {
        self.activated_at
    }

    /// Return the deprecation timestamp.
    #[must_use]
    pub const fn deprecated_at(&self) -> Timestamp {
        self.deprecated_at
    }

    /// Return the deprecation reason.
    #[must_use]
    pub const fn reason(&self) -> IStr {
        self.reason
    }

    /// Disable this deprecated pack and free its pack name.
    ///
    /// # Errors
    ///
    /// Returns [`ActivationError::SinkRefused`] when the sink rejects the
    /// disabled event.
    pub fn disable(
        self,
        sink: &dyn LifecycleSink,
        registry: &mut ActivationRegistry,
        now: Timestamp,
    ) -> Result<Disabled, ActivationError> {
        let event = LifecycleEvent::Disabled {
            pack_name: self.manifest.pack_name.clone(),
            principal: self.principal,
            at: now,
        };
        sink.record(&event)?;
        registry.remove(&self.manifest.pack_name);
        Ok(Disabled {
            manifest: self.manifest,
            content_hash: self.content_hash,
            principal: self.principal,
            staged_at: self.staged_at,
            activated_at: self.activated_at,
            deprecated_at: self.deprecated_at,
            disabled_at: now,
            reason: self.reason,
        })
    }
}

/// Disabled terminal lifecycle state.
#[derive(Clone, Debug)]
pub struct Disabled {
    manifest: ProcedurePackManifest,
    content_hash: ContentHash,
    principal: Principal,
    staged_at: Timestamp,
    activated_at: Timestamp,
    deprecated_at: Timestamp,
    disabled_at: Timestamp,
    reason: IStr,
}

impl Disabled {
    /// Return the pack name.
    #[must_use]
    pub fn pack_name(&self) -> &str {
        &self.manifest.pack_name
    }

    /// Return the validated manifest.
    #[must_use]
    pub const fn manifest(&self) -> &ProcedurePackManifest {
        &self.manifest
    }

    /// Return the pack content hash.
    #[must_use]
    pub const fn content_hash(&self) -> ContentHash {
        self.content_hash
    }

    /// Return the activation principal.
    #[must_use]
    pub const fn principal(&self) -> Principal {
        self.principal
    }

    /// Return the staging timestamp.
    #[must_use]
    pub const fn staged_at(&self) -> Timestamp {
        self.staged_at
    }

    /// Return the activation timestamp.
    #[must_use]
    pub const fn activated_at(&self) -> Timestamp {
        self.activated_at
    }

    /// Return the deprecation timestamp.
    #[must_use]
    pub const fn deprecated_at(&self) -> Timestamp {
        self.deprecated_at
    }

    /// Return the disabled timestamp.
    #[must_use]
    pub const fn disabled_at(&self) -> Timestamp {
        self.disabled_at
    }

    /// Return the deprecation reason carried into the terminal state.
    #[must_use]
    pub const fn reason(&self) -> IStr {
        self.reason
    }
}

fn extract_pack_name(raw_bytes: &[u8]) -> Option<String> {
    let value = serde_json::from_slice::<Value>(raw_bytes).ok()?;
    value
        .get("pack_name")
        .and_then(Value::as_str)
        .map(str::to_owned)
}
