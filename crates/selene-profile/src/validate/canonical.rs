//! Canonical semantic ordering used for stable serialization and hashing.

use crate::{ApplicabilityExpression, Profile};

pub(super) fn canonicalize(profile: &mut Profile) {
    profile.selected_features.sort();
    profile
        .clause_anchors
        .sort_by(|left, right| left.id.cmp(&right.id));
    profile
        .features
        .sort_by(|left, right| left.id.cmp(&right.id));
    for feature in &mut profile.features {
        feature.clause_anchors.sort();
        feature.evidence.sort();
    }
    profile
        .implications
        .sort_by(|left, right| left.id.cmp(&right.id));
    for implication in &mut profile.implications {
        implication.clause_anchors.sort();
        implication.evidence.sort();
    }
    profile
        .implementation_defined_choices
        .sort_by(|left, right| left.id.cmp(&right.id));
    for choice in &mut profile.implementation_defined_choices {
        choice.clause_anchors.sort();
        choice.evidence.sort();
    }
    profile
        .implementation_dependent_notes
        .sort_by(|left, right| left.id.cmp(&right.id));
    for note in &mut profile.implementation_dependent_notes {
        note.clause_anchors.sort();
        note.evidence.sort();
    }
    profile
        .implementation_extensions
        .sort_by(|left, right| left.id.cmp(&right.id));
    for extension in &mut profile.implementation_extensions {
        extension.clause_anchors.sort();
        extension.evidence.sort();
    }
    profile
        .evidence
        .sort_by(|left, right| left.id.cmp(&right.id));
    profile
        .applicability
        .sort_by(|left, right| left.id.cmp(&right.id));
    for definition in &mut profile.applicability {
        canonicalize_expression(&mut definition.expression);
    }
}

fn canonicalize_expression(expression: &mut ApplicabilityExpression) {
    match expression {
        ApplicabilityExpression::All { items } | ApplicabilityExpression::Any { items } => {
            for item in &mut *items {
                canonicalize_expression(item);
            }
            items.sort();
        }
        ApplicabilityExpression::Not { item } => canonicalize_expression(item),
        ApplicabilityExpression::Always
        | ApplicabilityExpression::Feature { .. }
        | ApplicabilityExpression::Extension { .. }
        | ApplicabilityExpression::ImplementationDefined { .. }
        | ApplicabilityExpression::Applicability { .. } => {}
    }
}
