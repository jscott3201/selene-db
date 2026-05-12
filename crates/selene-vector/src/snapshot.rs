//! Provider tags and section helpers for selene-vector snapshot participation.

use selene_graph::{ProviderTag, SubTag};

/// First-party provider tag reserved for selene-vector.
pub(crate) const VECT: ProviderTag = ProviderTag(*b"VECT");

/// HNSW graph topology section. Filled in BRIEF-61.
pub(crate) const GRPH: SubTag = SubTag(*b"GRPH");

/// Raw f32 vector payload section. Filled in BRIEF-61.
pub(crate) const VECS: SubTag = SubTag(*b"VECS");

/// Quantized vector payload section. Filled in BRIEF-63.
pub(crate) const QUNT: SubTag = SubTag(*b"QUNT");

/// Stable declared subsection order for the `VECT` provider.
pub(crate) const DECLARED_SUB_TAGS: [SubTag; 3] = [GRPH, VECS, QUNT];

/// Return true when `sub_tag` is one of the BRIEF-57 reserved sections.
pub(crate) fn is_declared(sub_tag: SubTag) -> bool {
    matches!(sub_tag, GRPH | VECS | QUNT)
}
