//! Analyzer type cells.

use std::collections::HashMap;

use crate::{GqlType, ValueExpr};

/// Stable, opaque identifier for a `ValueExpr` cell within one analyzer call.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct ExprId(u32);

impl ExprId {
    pub(crate) const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// Return this expression cell's zero-based numeric index.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Type carried by an analyzed expression cell.
///
/// `Dynamic` is the explicit sink for static-inference gaps. It is not a
/// hint downstream stages can ignore; planners and executors must handle it
/// deliberately until BRIEF-22 resolves more expressions statically.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AnalyzedType {
    /// Statically resolved type.
    Resolved(GqlType),
    /// Static inference did not resolve this expression in the current pass.
    Dynamic,
}

impl AnalyzedType {
    /// Canonical dynamic type cell used by BRIEF-21.
    pub const DYNAMIC: Self = Self::Dynamic;

    /// Return true when this type cell is dynamic.
    #[must_use]
    pub const fn is_dynamic(&self) -> bool {
        matches!(self, Self::Dynamic)
    }

    /// Return true when this type cell has a concrete static type.
    #[must_use]
    pub const fn is_resolved(&self) -> bool {
        matches!(self, Self::Resolved(_))
    }
}

/// Side-table of inferred type cells, indexed by [`ExprId`].
#[derive(Clone, Debug, Default)]
pub struct ExprTypeTable {
    cells: Vec<AnalyzedType>,
}

impl ExprTypeTable {
    pub(crate) fn push(&mut self, ty: AnalyzedType) -> ExprId {
        let id = ExprId::new(self.cells.len() as u32);
        self.cells.push(ty);
        id
    }

    /// Type cell for `id`.
    ///
    /// # Panics
    ///
    /// Panics if `id` was not produced by this table's allocator; that would
    /// indicate analyzer corruption rather than user input.
    #[must_use]
    pub fn get(&self, id: ExprId) -> &AnalyzedType {
        &self.cells[id.0 as usize]
    }

    /// Number of allocated cells.
    #[must_use]
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// Return true when the table has no expression cells.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Iterate over type cells in allocation order.
    pub fn iter(&self) -> impl Iterator<Item = (ExprId, &AnalyzedType)> {
        self.cells
            .iter()
            .enumerate()
            .map(|(index, ty)| (ExprId::new(index as u32), ty))
    }
}

/// Lookup table from parser AST expression nodes to their allocated [`ExprId`].
#[derive(Debug, Default)]
pub struct ExprIdMap {
    // Why: the parser AST is moved into `AnalyzedStatement` without relocating
    // its heap-backed expression nodes (`Vec` buffers and `Box` payloads stay
    // put). The map is only valid for the owning `AnalyzedStatement`, so it is
    // intentionally not Clone.
    ids: HashMap<*const ValueExpr, ExprId>,
}

impl ExprIdMap {
    pub(crate) fn insert(&mut self, expr: &ValueExpr, id: ExprId) {
        self.ids.insert(std::ptr::from_ref(expr), id);
    }

    /// Return the expression ID allocated for `expr`.
    #[must_use]
    pub fn get(&self, expr: &ValueExpr) -> Option<ExprId> {
        self.ids.get(&std::ptr::from_ref(expr)).copied()
    }

    /// Number of mapped expression nodes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// Return true when the map has no expression IDs.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
}
