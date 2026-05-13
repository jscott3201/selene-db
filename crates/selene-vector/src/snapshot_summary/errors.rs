//! Stable VectorError rendering for snapshot fixtures.

use crate::VectorError;

use super::{format_score, op_name, vector_error_kind_for};

/// Render a `VectorError` without using `Debug` or `Display` passthrough.
#[must_use]
pub fn render_vector_error(error: &VectorError) -> String {
    let kind = vector_error_kind_for(error).name();
    match error {
        VectorError::InvalidConfig { reason } => {
            format!("{kind}{{reason={reason:?}}}")
        }
        VectorError::DimensionMismatch { expected, observed } => {
            format!("{kind}{{expected={expected}, observed={observed}}}")
        }
        VectorError::SectionDecodeFailed { sub_tag, reason } => {
            format!("{kind}{{sub_tag={sub_tag}, reason={reason:?}}}")
        }
        VectorError::SectionEncodeFailed { sub_tag, reason } => {
            format!("{kind}{{sub_tag={sub_tag}, reason={reason:?}}}")
        }
        VectorError::InvalidNodeId { node_id, reason } => {
            format!("{kind}{{node_id=n{}, reason={reason:?}}}", node_id.get())
        }
        VectorError::DimensionsLocked { expected, observed } => {
            format!("{kind}{{expected={expected}, observed={observed}}}")
        }
        VectorError::InvalidPayload { reason } => {
            format!("{kind}{{reason={reason:?}}}")
        }
        VectorError::EncodeFailed { reason } => {
            format!("{kind}{{reason={reason:?}}}")
        }
        VectorError::OperationNotSupportedYet { op, node_id, brief } => {
            format!(
                "{kind}{{op={}, node_id=n{}, brief={brief:?}}}",
                op_name(*op),
                node_id.get()
            )
        }
        VectorError::DuplicateNodeId { node_id } => {
            format!("{kind}{{node_id=n{}}}", node_id.get())
        }
        VectorError::NonFiniteVectorComponent {
            node_id,
            index,
            value,
        } => {
            format!(
                "{kind}{{node_id=n{}, index={index}, value={}}}",
                node_id.get(),
                format_score(*value)
            )
        }
        VectorError::InternalIndexExhausted { current } => {
            format!("{kind}{{current={current}}}")
        }
        VectorError::MaxLayerExceedsCap { observed, cap } => {
            format!("{kind}{{observed={observed}, cap={cap}}}")
        }
        VectorError::NonFiniteQueryComponent { index, value } => {
            format!("{kind}{{index={index}, value={}}}", format_score(*value))
        }
        VectorError::PqTrainingDeferred {
            observed_vectors,
            required,
        } => format!("{kind}{{observed_vectors={observed_vectors}, required={required}}}"),
        VectorError::PqDimensionNotDivisible { dim, m_subspaces } => {
            format!("{kind}{{dim={dim}, m_subspaces={m_subspaces}}}")
        }
        VectorError::IvfTrainingDeferred {
            observed_vectors,
            required,
        } => format!("{kind}{{observed_vectors={observed_vectors}, required={required}}}"),
        VectorError::IvfDimensionMismatch { expected, observed } => {
            format!("{kind}{{expected={expected}, observed={observed}}}")
        }
        VectorError::IvfInvalidNProbe { n_probe, k_coarse } => {
            format!("{kind}{{n_probe={n_probe}, k_coarse={k_coarse}}}")
        }
        VectorError::IvfSectionInconsistent { reason } => {
            format!("{kind}{{reason={reason:?}}}")
        }
        VectorError::IvfTrainingFailed { context, reason } => {
            format!("{kind}{{context={context:?}, reason={reason:?}}}")
        }
        VectorError::PqCodebookTrainFailed { context, reason } => {
            format!("{kind}{{context={context:?}, reason={reason:?}}}")
        }
        VectorError::OpqTrainingFailed { context, reason } => {
            format!("{kind}{{context={context:?}, reason={reason:?}}}")
        }
    }
}
