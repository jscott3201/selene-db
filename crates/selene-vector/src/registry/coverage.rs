use std::collections::BTreeSet;
use std::sync::Arc;

use selene_graph::{ProviderError, SubTag};

pub(super) fn validate_snapshot_wrapper_coverage<'a>(
    sub_tag: SubTag,
    staged_names: impl IntoIterator<Item = Arc<str>>,
    wrapper_names: impl IntoIterator<Item = &'a str>,
) -> Result<(), ProviderError> {
    let staged = staged_names
        .into_iter()
        .map(|name| name.to_string())
        .collect::<BTreeSet<_>>();
    let wrapper = wrapper_names
        .into_iter()
        .map(ToOwned::to_owned)
        .collect::<BTreeSet<_>>();
    if let Some(name) = staged.difference(&wrapper).next() {
        return Err(ProviderError::InvalidPayload {
            reason: format!("{sub_tag} wrapper missing entry '{name}'"),
        });
    }
    if let Some(name) = wrapper.difference(&staged).next() {
        return Err(ProviderError::InvalidPayload {
            reason: format!("{sub_tag} wrapper has unexpected entry '{name}'"),
        });
    }
    Ok(())
}
