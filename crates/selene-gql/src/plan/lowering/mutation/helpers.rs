//! Lowering helpers for the mutation plan builder: insert-site
//! classification, endpoint/binding resolution, output-column bookkeeping,
//! and the insert-site id allocator. Split from the parent builder file to
//! keep both under the repository 700-LOC cap; reached only through
//! `super::mutation`.

use super::*;

pub(super) fn classify_insert_element(
    element: &PatternElement,
    analyzed: &AnalyzedStatement,
) -> Result<InsertSiteEmission, PlannerError> {
    let (name, span, expected) = match element {
        PatternElement::Node(node) => {
            (node.binding.clone(), node.span, BindingDeclKind::InsertNode)
        }
        PatternElement::Edge(edge) => {
            (edge.binding.clone(), edge.span, BindingDeclKind::InsertEdge)
        }
    };
    let Some(name) = name else {
        return Ok(InsertSiteEmission::Emitted);
    };
    if fresh_insert_binding(name.clone(), span, expected, analyzed).is_some() {
        return Ok(InsertSiteEmission::Emitted);
    }
    if pattern_reuse_binding(name, span, analyzed).is_some() {
        return Ok(InsertSiteEmission::Skipped);
    }
    Err(mismatch(span))
}

pub(super) fn endpoint_ref(
    element: &PatternElement,
    index: usize,
    sites: &HashMap<usize, InsertSiteId>,
    analyzed: &AnalyzedStatement,
    visible: &[BindingTableColumn],
    span: SourceSpan,
) -> Result<InsertEndpointRef, PlannerError> {
    let PatternElement::Node(node) = element else {
        return Err(mismatch(span));
    };
    if let Some(binding) = named_node_endpoint(node, analyzed)? {
        return Ok(InsertEndpointRef::Binding {
            binding,
            column_index: binding_column_index(binding, analyzed, visible)?,
        });
    }
    sites
        .get(&index)
        .copied()
        .map(InsertEndpointRef::InsertedNode)
        .ok_or_else(|| mismatch(span))
}

pub(super) fn named_node_endpoint(
    node: &NodePattern,
    analyzed: &AnalyzedStatement,
) -> Result<Option<BindingId>, PlannerError> {
    let Some(ref name) = node.binding else {
        return Ok(None);
    };
    if let Some(binding) = pattern_reuse_binding(name.clone(), node.span, analyzed) {
        return Ok(Some(binding));
    }
    if let Some(binding) = fresh_insert_binding(
        name.clone(),
        node.span,
        BindingDeclKind::InsertNode,
        analyzed,
    ) {
        return Ok(Some(binding));
    }
    Err(mismatch(node.span))
}

pub(super) fn fresh_insert_binding(
    name: selene_core::DbString,
    span: SourceSpan,
    kind: BindingDeclKind,
    analyzed: &AnalyzedStatement,
) -> Option<BindingId> {
    analyzed
        .scopes
        .declarations()
        .iter()
        .find(|decl| decl.name() == name && decl.span() == span && decl.kind() == kind)
        .map(|decl| decl.id())
}

pub(super) fn pattern_reuse_binding(
    name: selene_core::DbString,
    span: SourceSpan,
    analyzed: &AnalyzedStatement,
) -> Option<BindingId> {
    analyzed
        .references
        .iter()
        .find(|reference| {
            reference.name == name
                && reference.span == span
                && reference.kind == BindingUseKind::PatternReuse
        })
        .map(|reference| reference.binding)
}

pub(super) fn visible_column_for_binding(
    binding: BindingId,
    analyzed: &AnalyzedStatement,
) -> Result<BindingTableColumn, PlannerError> {
    let declaration =
        analyzed
            .scopes
            .declaration(binding)
            .ok_or(PlannerError::BindingResolutionLost {
                binding,
                span: analyzed.span,
            })?;
    Ok(BindingTableColumn {
        name: Some(declaration.name()),
        hidden: None,
        ty: declaration.ty().clone(),
    })
}

pub(super) fn push_insert_output(
    binding: Option<BindingId>,
    analyzed: &AnalyzedStatement,
    visible: &mut Vec<BindingTableColumn>,
) -> Result<(Option<u32>, Option<BindingTableColumn>), PlannerError> {
    let Some(binding) = binding else {
        return Ok((None, None));
    };
    let index = u32::try_from(visible.len()).expect("binding table column count fits u32");
    let column = visible_column_for_binding(binding, analyzed)?;
    visible.push(column.clone());
    Ok((Some(index), Some(column)))
}

pub(super) fn binding_column_index(
    binding: BindingId,
    analyzed: &AnalyzedStatement,
    visible: &[BindingTableColumn],
) -> Result<u32, PlannerError> {
    let declaration =
        analyzed
            .scopes
            .declaration(binding)
            .ok_or(PlannerError::BindingResolutionLost {
                binding,
                span: analyzed.span,
            })?;
    let index = visible
        .iter()
        .position(|column| column.name == Some(declaration.name()))
        .ok_or(PlannerError::BindingResolutionLost {
            binding,
            span: declaration.span(),
        })?;
    Ok(u32::try_from(index).expect("binding table column count fits u32"))
}

pub(super) fn mutation_target_column_index(
    binding: BindingId,
    element: ElementKind,
    analyzed: &AnalyzedStatement,
    visible: &[BindingTableColumn],
) -> Result<u32, PlannerError> {
    match element {
        ElementKind::Node | ElementKind::Edge => binding_column_index(binding, analyzed, visible),
        ElementKind::Path | ElementKind::Alias => Ok(0),
    }
}

pub(super) fn consume_entry(
    write_set: &MutationWriteSet,
    cursor: &mut usize,
    expected_span: SourceSpan,
) -> Result<WriteSetEntry, PlannerError> {
    let entry = write_set
        .entries
        .get(*cursor)
        .cloned()
        .ok_or_else(|| mismatch(expected_span))?;
    if entry.span != expected_span {
        return Err(mismatch(entry.span));
    }
    *cursor += 1;
    Ok(entry)
}

pub(super) fn mismatch(span: SourceSpan) -> PlannerError {
    PlannerError::WriteSetPatternMismatch { span }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum InsertSiteEmission {
    Emitted,
    Skipped,
}

#[derive(Default)]
pub(super) struct InsertSiteIdAlloc {
    next: u32,
}

impl InsertSiteIdAlloc {
    pub(super) fn alloc(&mut self) -> InsertSiteId {
        let id = InsertSiteId::new(self.next);
        self.next += 1;
        id
    }
}
