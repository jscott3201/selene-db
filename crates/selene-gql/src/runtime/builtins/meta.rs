//! Static metadata builders for the native platform built-ins.
//!
//! Ported verbatim from the historical procedure-pack built-in module
//! (`StaticParameter`, `StaticOutputColumn`) plus the pack registry's
//! `parameter` / `output_column` storage conversion. Keeping the
//! `with_description` / `with_default_doc` / `with_default` shape identical means
//! the relocated built-ins expose byte-identical `SHOW PROCEDURES` metadata.

use selene_core::intern;

use crate::{GqlType, ProcedureDefaultValue, ProcedureOutputColumn, ProcedureParameter};

/// Static parameter metadata exposed by a built-in.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct StaticParameter {
    /// Parameter name.
    pub(super) name: &'static str,
    /// Parameter type.
    pub(super) ty: GqlType,
    /// Whether NULL is accepted.
    pub(super) nullable: bool,
    /// Human-readable parameter description.
    pub(super) description: &'static str,
    /// Documentation-only default value text.
    pub(super) default_doc: Option<&'static str>,
    /// Executable default value for omitted trailing optional arguments.
    pub(super) default: Option<ProcedureDefaultValue>,
}

impl StaticParameter {
    /// Construct static parameter metadata.
    pub(super) const fn new(name: &'static str, ty: GqlType, nullable: bool) -> Self {
        Self {
            name,
            ty,
            nullable,
            description: "",
            default_doc: None,
            default: None,
        }
    }

    /// Attach a human-readable parameter description.
    pub(super) const fn with_description(mut self, description: &'static str) -> Self {
        self.description = description;
        self
    }

    /// Attach documentation-only default value text.
    pub(super) const fn with_default_doc(mut self, default_doc: &'static str) -> Self {
        self.default_doc = Some(default_doc);
        self
    }

    /// Attach an executable default value.
    pub(super) const fn with_default(mut self, default: ProcedureDefaultValue) -> Self {
        self.default = Some(default);
        self
    }

    /// Convert into planner-visible parameter metadata, preserving the pack's
    /// `description` / `default_doc` / `default` carry-over rules.
    pub(super) fn into_parameter(self) -> ProcedureParameter {
        let mut result = ProcedureParameter::new(intern_static(self.name), self.ty, self.nullable)
            .with_description(self.description);
        if let Some(default_doc) = self.default_doc {
            result = result.with_default_doc(default_doc);
        }
        if let Some(default) = self.default {
            result = result.with_default(default);
        }
        result
    }
}

/// Static output-column metadata exposed by a built-in.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct StaticOutputColumn {
    /// Output column name.
    pub(super) name: &'static str,
    /// Output column type.
    pub(super) ty: GqlType,
    /// Human-readable output-column description.
    pub(super) description: &'static str,
}

impl StaticOutputColumn {
    /// Construct static output-column metadata.
    pub(super) const fn new(name: &'static str, ty: GqlType) -> Self {
        Self {
            name,
            ty,
            description: "",
        }
    }

    /// Attach a human-readable output-column description.
    pub(super) const fn with_description(mut self, description: &'static str) -> Self {
        self.description = description;
        self
    }

    /// Convert into planner-visible output-column metadata.
    pub(super) fn into_output_column(self) -> ProcedureOutputColumn {
        ProcedureOutputColumn::new(intern_static(self.name), self.ty)
            .with_description(self.description)
    }
}

/// Intern a static built-in metadata name.
///
/// # Panics
///
/// Panics only if the name exceeds the per-string byte cap (IL013). Built-in
/// names/columns are a fixed, compile-time set of short identifiers, so this
/// never fires in practice; the native registry is built once at engine
/// startup.
fn intern_static(value: &'static str) -> selene_core::IStr {
    intern(value).expect("static built-in metadata name interns")
}
