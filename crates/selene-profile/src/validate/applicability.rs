//! Applicability evaluation against selected features and extensions.

use std::collections::{BTreeMap, BTreeSet};

use crate::{ApplicabilityExpression, Profile, RuntimeSupport, closure::ClosureGraph};

use super::{MAX_APPLICABILITY_DEPTH, ProfileError, invalid};

pub(super) fn evaluate(
    profile: &Profile,
    closure: &ClosureGraph,
) -> Result<BTreeMap<String, bool>, ProfileError> {
    let definitions = profile
        .applicability
        .iter()
        .map(|definition| (definition.id.as_str(), &definition.expression))
        .collect::<BTreeMap<_, _>>();
    let selected = closure.closure_for(profile.selected_features.iter().map(|id| id.as_str()));
    let extensions = profile
        .implementation_extensions
        .iter()
        .filter(|extension| extension.runtime_support == RuntimeSupport::Supported)
        .map(|extension| extension.id.as_str())
        .collect::<BTreeSet<_>>();
    let choices = profile
        .implementation_defined_choices
        .iter()
        .map(|choice| choice.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut output = BTreeMap::new();
    for id in definitions.keys().copied() {
        let value = evaluate_named(id, 0, &definitions, &selected, &extensions, &choices)?;
        output.insert(id.to_owned(), value);
    }
    Ok(output)
}

fn evaluate_named(
    id: &str,
    depth: usize,
    definitions: &BTreeMap<&str, &ApplicabilityExpression>,
    selected: &BTreeSet<String>,
    extensions: &BTreeSet<&str>,
    choices: &BTreeSet<&str>,
) -> Result<bool, ProfileError> {
    let expression = definitions
        .get(id)
        .expect("applicability references were validated");
    evaluate_expression(
        expression,
        depth,
        definitions,
        selected,
        extensions,
        choices,
    )
}

fn evaluate_expression(
    expression: &ApplicabilityExpression,
    depth: usize,
    definitions: &BTreeMap<&str, &ApplicabilityExpression>,
    selected: &BTreeSet<String>,
    extensions: &BTreeSet<&str>,
    choices: &BTreeSet<&str>,
) -> Result<bool, ProfileError> {
    if depth > MAX_APPLICABILITY_DEPTH {
        return Err(invalid(format!(
            "applicability evaluation exceeds depth {MAX_APPLICABILITY_DEPTH}"
        )));
    }
    let next = depth + 1;
    match expression {
        ApplicabilityExpression::Always => Ok(true),
        ApplicabilityExpression::Feature { feature_id } => {
            Ok(selected.contains(feature_id.as_str()))
        }
        ApplicabilityExpression::Extension { extension_id } => {
            Ok(extensions.contains(extension_id.as_str()))
        }
        ApplicabilityExpression::ImplementationDefined { choice_id } => {
            Ok(choices.contains(choice_id.as_str()))
        }
        ApplicabilityExpression::Applicability { applicability_id } => evaluate_named(
            applicability_id.as_str(),
            next,
            definitions,
            selected,
            extensions,
            choices,
        ),
        ApplicabilityExpression::All { items } => {
            for item in items {
                if !evaluate_expression(item, next, definitions, selected, extensions, choices)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        ApplicabilityExpression::Any { items } => {
            for item in items {
                if evaluate_expression(item, next, definitions, selected, extensions, choices)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        ApplicabilityExpression::Not { item } => Ok(!evaluate_expression(
            item,
            next,
            definitions,
            selected,
            extensions,
            choices,
        )?),
    }
}
