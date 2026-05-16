//! Provider tags and section helpers for selene-vector snapshot participation.

use selene_graph::{ProviderTag, SubTag};

/// First-party provider tag reserved for selene-vector.
pub(crate) const VECT: ProviderTag = ProviderTag(*b"VECT");

/// Provider tag reserved for the IVF-PQ vector index provider.
pub(crate) const IVFP: ProviderTag = ProviderTag(*b"IVFP");

/// HNSW graph topology section.
pub(crate) const GRPH: SubTag = SubTag(*b"GRPH");

/// Raw f32 vector payload section.
pub(crate) const VECS: SubTag = SubTag(*b"VECS");

/// Quantized vector payload section.
pub(crate) const QUNT: SubTag = SubTag(*b"QUNT");

/// Coarse-quantizer section tag for the `IVFP` provider.
pub(crate) const CQNT: SubTag = SubTag(*b"CQNT");

/// Residual PQ codebook section tag for the `IVFP` provider.
pub(crate) const IPQB: SubTag = SubTag(*b"IPQB");

/// Posting-list section tag for the `IVFP` provider.
pub(crate) const POST: SubTag = SubTag(*b"POST");

/// Stable declared subsection order for the `VECT` provider.
pub(crate) const DECLARED_SUB_TAGS: [SubTag; 3] = [GRPH, VECS, QUNT];

/// Stable declared subsection order for the `IVFP` provider.
pub(crate) const DECLARED_SUB_TAGS_IVF: [SubTag; 3] = [CQNT, IPQB, POST];

/// Return true when `sub_tag` is one of the reserved sections.
pub(crate) fn is_declared(sub_tag: SubTag) -> bool {
    matches!(sub_tag, GRPH | VECS | QUNT)
}

/// Return true when `sub_tag` belongs to the `IVFP` provider.
pub(crate) fn is_declared_ivf(sub_tag: SubTag) -> bool {
    matches!(sub_tag, CQNT | IPQB | POST)
}
