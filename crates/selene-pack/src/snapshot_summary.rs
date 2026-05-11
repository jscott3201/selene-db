//! Deterministic rendering for procedure-pack snapshot fixtures.

use std::fmt;

use selene_core::Value;

use crate::manifest::{canonical_bytes_for_hashing, hex_lower};
use crate::{
    ActivationError, ActivationRegistry, ActivationStatus, ContentHash, LifecycleEvent,
    ManifestError, ProcedurePackManifest,
};

const CANONICAL_PREVIEW_HEX_CHARS: usize = 256;

/// Input accepted by [`pack_summary`].
pub struct PackSnapshotInput<'a> {
    /// Fixture result to summarize.
    pub fixture_kind: PackFixtureKind<'a>,
}

/// Result shape produced by one pack snapshot fixture.
#[non_exhaustive]
pub enum PackFixtureKind<'a> {
    /// Manifest parse result.
    ManifestParse {
        /// Borrowed parse result.
        result: Result<&'a ProcedurePackManifest, &'a ManifestError>,
    },
    /// Activation lifecycle run.
    LifecycleRun {
        /// Events captured by the fixture sink.
        events: &'a [LifecycleEvent],
        /// Final activation registry state.
        final_registry: &'a ActivationRegistry,
        /// Terminal error, when the script intentionally exercises a failure.
        error: Option<&'a ActivationError>,
    },
    /// Canonical content-hash fixture.
    HashCanonical {
        /// Parsed manifest whose canonical bytes are summarized.
        manifest: &'a ProcedurePackManifest,
    },
    /// Rows projected from `selene.pack.history`.
    HistoryRows {
        /// Already-projected history rows.
        rows: &'a [Vec<Value>],
    },
}

/// Stable procedure-pack snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackSnapshot {
    /// Rendered sections.
    pub sections: Vec<PackSnapshotSection>,
}

/// One rendered snapshot section.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackSnapshotSection {
    /// Section title.
    pub title: &'static str,
    /// Section body lines.
    pub lines: Vec<String>,
}

/// Build a stable snapshot summary for one pack corpus fixture.
#[must_use]
pub fn pack_summary(input: &PackSnapshotInput<'_>) -> PackSnapshot {
    match &input.fixture_kind {
        PackFixtureKind::ManifestParse { result } => manifest_summary(result),
        PackFixtureKind::LifecycleRun {
            events,
            final_registry,
            error,
        } => lifecycle_summary(events, final_registry, *error),
        PackFixtureKind::HashCanonical { manifest } => hash_summary(manifest),
        PackFixtureKind::HistoryRows { rows } => history_summary(rows),
    }
}

impl fmt::Display for PackSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, section) in self.sections.iter().enumerate() {
            if index > 0 {
                writeln!(formatter)?;
            }
            writeln!(formatter, "{}:", section.title)?;
            if section.lines.is_empty() {
                writeln!(formatter, "- <empty>")?;
            } else {
                for line in &section.lines {
                    writeln!(formatter, "- {line}")?;
                }
            }
        }
        Ok(())
    }
}

fn manifest_summary(result: &Result<&ProcedurePackManifest, &ManifestError>) -> PackSnapshot {
    let lines = match result {
        Ok(manifest) => {
            let mut procedure_names = manifest
                .procedures
                .iter()
                .map(|procedure| procedure.name.as_str())
                .collect::<Vec<_>>();
            procedure_names.sort_unstable();
            vec![format!(
                "OK pack_name={} pack_version={} content_hash={} procedure_count={} procedure_names=[{}]",
                quoted(&manifest.pack_name),
                quoted(&manifest.pack_version),
                quoted(&manifest.content_hash),
                manifest.procedures.len(),
                procedure_names
                    .into_iter()
                    .map(quoted)
                    .collect::<Vec<_>>()
                    .join(", ")
            )]
        }
        Err(error) => vec![render_manifest_error(error)],
    };
    PackSnapshot {
        sections: vec![PackSnapshotSection {
            title: "manifest",
            lines,
        }],
    }
}

fn lifecycle_summary(
    events: &[LifecycleEvent],
    registry: &ActivationRegistry,
    error: Option<&ActivationError>,
) -> PackSnapshot {
    let event_lines = events
        .iter()
        .enumerate()
        .map(|(index, event)| format!("[{index}] {}", render_lifecycle_event(event)))
        .collect::<Vec<_>>();
    let registry_lines = registry
        .iter()
        .map(|entry| {
            let status = match entry.status() {
                ActivationStatus::Active => "Active",
                ActivationStatus::Deprecated => "Deprecated",
            };
            format!(
                "{} -> status={} content_hash={} principal={} activated_at={} deprecated_at={}",
                quoted(entry.pack_name()),
                status,
                quoted(&content_hash_string(entry.content_hash())),
                quoted(entry.principal().as_istr().as_str()),
                timestamp(entry.activated_at()),
                entry
                    .deprecated_at()
                    .map_or_else(|| "NULL".to_owned(), timestamp)
            )
        })
        .collect::<Vec<_>>();
    let error_lines = error.map_or_else(Vec::new, |error| vec![render_activation_error(error)]);
    PackSnapshot {
        sections: vec![
            PackSnapshotSection {
                title: "events",
                lines: event_lines,
            },
            PackSnapshotSection {
                title: "registry",
                lines: registry_lines,
            },
            PackSnapshotSection {
                title: "error",
                lines: error_lines,
            },
        ],
    }
}

fn hash_summary(manifest: &ProcedurePackManifest) -> PackSnapshot {
    let canonical = canonical_bytes_for_hashing(manifest);
    let canonical_hex = hex_lower(&canonical);
    let preview = canonical_hex
        .chars()
        .take(CANONICAL_PREVIEW_HEX_CHARS)
        .collect::<String>();
    let hash = ContentHash::from_validated_manifest(manifest);
    PackSnapshot {
        sections: vec![PackSnapshotSection {
            title: "hash",
            lines: vec![
                format!("CANONICAL_BYTES_LEN {}", canonical.len()),
                format!("CANONICAL_BYTES_HEX_PREVIEW {preview}"),
                format!("CONTENT_HASH {}", quoted(&content_hash_string(hash))),
            ],
        }],
    }
}

fn history_summary(rows: &[Vec<Value>]) -> PackSnapshot {
    PackSnapshot {
        sections: vec![PackSnapshotSection {
            title: "history",
            lines: rows
                .iter()
                .enumerate()
                .map(|(index, row)| format!("[{index}] {}", render_history_row(row)))
                .collect(),
        }],
    }
}

fn render_manifest_error(error: &ManifestError) -> String {
    let mut fields = Vec::new();
    match error {
        ManifestError::InvalidJson { .. }
        | ManifestError::SchemaViolation { .. }
        | ManifestError::DeserializeError { .. } => {}
        ManifestError::UnsupportedSchemaVersion { found, supported } => {
            fields.push(format!("found={found}"));
            fields.push(format!("supported={supported}"));
        }
        ManifestError::InvalidContentHashFormat { content_hash } => {
            fields.push(format!("content_hash={}", quoted(content_hash)));
        }
        ManifestError::ContentHashMismatch { declared, computed } => {
            fields.push(format!("declared={}", quoted(declared)));
            fields.push(format!("computed={}", quoted(computed)));
        }
        ManifestError::InvalidPackVersion { pack_version, .. } => {
            fields.push(format!("pack_version={}", quoted(pack_version)));
        }
        ManifestError::InvalidPackName { pack_name, .. } => {
            fields.push(format!("pack_name={}", quoted(pack_name)));
        }
        ManifestError::InvalidProcedureName { procedure_name, .. } => {
            fields.push(format!("name={}", quoted(procedure_name)));
        }
        ManifestError::ProcedureNameOutsidePack {
            procedure_name,
            expected_prefix,
        } => {
            fields.push(format!("name={}", quoted(procedure_name)));
            fields.push(format!("expected_prefix={}", quoted(expected_prefix)));
        }
        ManifestError::ReservedNamespaceConflict { procedure_name } => {
            fields.push(format!("name={}", quoted(procedure_name)));
        }
        ManifestError::DuplicateProcedureName { procedure_name } => {
            fields.push(format!("name={}", quoted(procedure_name)));
        }
        ManifestError::MutabilityTierMismatch {
            procedure_name,
            declared_tier,
            declared_mutability,
            expected_tier,
        } => {
            fields.push(format!("name={}", quoted(procedure_name)));
            fields.push(format!("tier={declared_tier:?}"));
            fields.push(format!("mutability={declared_mutability:?}"));
            fields.push(format!("expected_tier={expected_tier:?}"));
        }
        ManifestError::PersistTierInManifest { procedure_name } => {
            fields.push(format!("name={}", quoted(procedure_name)));
        }
        ManifestError::InvalidInlineSchema {
            procedure_name,
            field,
            ..
        } => {
            fields.push(format!("name={}", quoted(procedure_name)));
            fields.push(format!("field={}", quoted(field)));
        }
        ManifestError::InvalidSchemaPath {
            procedure_name,
            field,
            path,
            ..
        } => {
            fields.push(format!("name={}", quoted(procedure_name)));
            fields.push(format!("field={}", quoted(field)));
            fields.push(format!("path={}", quoted(path)));
        }
        ManifestError::PackProcedureCountExceedsBound { count, limit } => {
            fields.push(format!("found={count}"));
            fields.push(format!("max={limit}"));
        }
        ManifestError::InlineSchemaSizeExceedsBound {
            procedure_name,
            field,
            size,
            limit,
        } => {
            fields.push(format!("name={}", quoted(procedure_name)));
            fields.push(format!("field={}", quoted(field)));
            fields.push(format!("found={size}"));
            fields.push(format!("max={limit}"));
        }
        ManifestError::ProcedureSchemaCompileFailed {
            procedure_name,
            field,
            ..
        } => {
            fields.push(format!("name={}", quoted(procedure_name)));
            fields.push(format!("field={}", quoted(field)));
        }
        ManifestError::ProcedureCapabilityMalformed {
            procedure_name,
            capability,
            ..
        } => {
            fields.push(format!("name={}", quoted(procedure_name)));
            fields.push(format!("capability={}", quoted(capability)));
        }
        ManifestError::ProcedureNameTooLong {
            procedure_name,
            length,
            limit,
        } => {
            fields.push(format!("name={}", quoted(procedure_name)));
            fields.push(format!("found={length}"));
            fields.push(format!("max={limit}"));
        }
    }
    let suffix = if fields.is_empty() {
        String::new()
    } else {
        format!(" {}", fields.join(" "))
    };
    format!(
        "ERR variant={} gate={}{}",
        manifest_error_variant(error),
        error.gate().id(),
        suffix
    )
}

fn manifest_error_variant(error: &ManifestError) -> &'static str {
    match error {
        ManifestError::InvalidJson { .. } => "InvalidJson",
        ManifestError::SchemaViolation { .. } => "SchemaViolation",
        ManifestError::DeserializeError { .. } => "DeserializeError",
        ManifestError::UnsupportedSchemaVersion { .. } => "UnsupportedSchemaVersion",
        ManifestError::InvalidContentHashFormat { .. } => "InvalidContentHashFormat",
        ManifestError::ContentHashMismatch { .. } => "ContentHashMismatch",
        ManifestError::InvalidPackVersion { .. } => "InvalidPackVersion",
        ManifestError::InvalidPackName { .. } => "InvalidPackName",
        ManifestError::InvalidProcedureName { .. } => "InvalidProcedureName",
        ManifestError::ProcedureNameOutsidePack { .. } => "ProcedureNameOutsidePack",
        ManifestError::ReservedNamespaceConflict { .. } => "ReservedNamespaceConflict",
        ManifestError::DuplicateProcedureName { .. } => "DuplicateProcedureName",
        ManifestError::MutabilityTierMismatch { .. } => "MutabilityTierMismatch",
        ManifestError::PersistTierInManifest { .. } => "PersistTierInManifest",
        ManifestError::InvalidInlineSchema { .. } => "InvalidInlineSchema",
        ManifestError::InvalidSchemaPath { .. } => "InvalidSchemaPath",
        ManifestError::PackProcedureCountExceedsBound { .. } => "PackProcedureCountExceedsBound",
        ManifestError::InlineSchemaSizeExceedsBound { .. } => "InlineSchemaSizeExceedsBound",
        ManifestError::ProcedureSchemaCompileFailed { .. } => "ProcedureSchemaCompileFailed",
        ManifestError::ProcedureCapabilityMalformed { .. } => "ProcedureCapabilityMalformed",
        ManifestError::ProcedureNameTooLong { .. } => "ProcedureNameTooLong",
    }
}

fn render_lifecycle_event(event: &LifecycleEvent) -> String {
    match event {
        LifecycleEvent::ValidationFailed {
            pack_name,
            principal,
            error,
            at,
        } => format!(
            "kind=validation_failed pack_name={} principal={} error={} at={}",
            pack_name.as_deref().map_or("NULL".to_owned(), quoted),
            quoted(principal.as_istr().as_str()),
            quoted(error),
            timestamp(*at)
        ),
        LifecycleEvent::Staged {
            pack_name,
            content_hash,
            principal,
            at,
        } => format!(
            "kind=staged pack_name={} content_hash={} principal={} at={}",
            quoted(pack_name),
            quoted(&content_hash_string(*content_hash)),
            quoted(principal.as_istr().as_str()),
            timestamp(*at)
        ),
        LifecycleEvent::Activated {
            pack_name,
            content_hash,
            principal,
            at,
        } => format!(
            "kind=activated pack_name={} content_hash={} principal={} at={}",
            quoted(pack_name),
            quoted(&content_hash_string(*content_hash)),
            quoted(principal.as_istr().as_str()),
            timestamp(*at)
        ),
        LifecycleEvent::Deprecated {
            pack_name,
            reason,
            principal,
            at,
        } => format!(
            "kind=deprecated pack_name={} reason={} principal={} at={}",
            quoted(pack_name),
            quoted(reason.as_str()),
            quoted(principal.as_istr().as_str()),
            timestamp(*at)
        ),
        LifecycleEvent::Disabled {
            pack_name,
            principal,
            at,
        } => format!(
            "kind=disabled pack_name={} principal={} at={}",
            quoted(pack_name),
            quoted(principal.as_istr().as_str()),
            timestamp(*at)
        ),
    }
}

fn render_activation_error(error: &ActivationError) -> String {
    match error {
        ActivationError::Manifest { source } => format!(
            "variant=Manifest gate={} manifest_variant={}",
            source.gate().id(),
            manifest_error_variant(source)
        ),
        ActivationError::PackAlreadyActive { pack_name } => format!(
            "variant=PackAlreadyActive gate={} pack_name={}",
            error.gate().map_or("none", |gate| gate.id()),
            quoted(pack_name)
        ),
        ActivationError::SinkRefused { detail } => {
            format!("variant=SinkRefused gate=none detail={}", quoted(detail))
        }
    }
}

fn render_history_row(row: &[Value]) -> String {
    let mut columns = Vec::new();
    let names = [
        "kind",
        "pack_name",
        "content_hash",
        "principal",
        "reason",
        "error",
        "at",
    ];
    for (name, value) in names.into_iter().zip(row.iter()) {
        columns.push(format!("{name}={}", render_value(value)));
    }
    columns.join(" ")
}

fn render_value(value: &Value) -> String {
    match value {
        Value::String(value) => quoted(value.as_str()),
        Value::ExternalString(value) => quoted(value.as_ref()),
        Value::ZonedDateTime(value) => timestamp(value.timestamp()),
        Value::Null => "NULL".to_owned(),
        other => format!("{other:?}"),
    }
}

fn content_hash_string(hash: ContentHash) -> String {
    format!("blake3:{}", hex_lower(&hash.as_bytes()))
}

fn timestamp(value: jiff::Timestamp) -> String {
    value.to_string()
}

fn quoted(value: &str) -> String {
    format!("{value:?}")
}
