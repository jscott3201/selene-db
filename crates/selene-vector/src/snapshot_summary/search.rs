//! Search-result rendering for vector snapshot fixtures.

use selene_core::NodeId;

use super::{format_score, render_node_id};

/// Stable search rows in score order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchRowsSummary {
    /// Rows formatted as `(NodeId, score)`.
    pub rows: Vec<(NodeId, ScoreBits)>,
}

impl SearchRowsSummary {
    /// Convert raw f32 score rows into a stable summary.
    #[must_use]
    pub fn from_rows(rows: &[(NodeId, f32)]) -> Self {
        Self {
            rows: rows
                .iter()
                .map(|(node_id, score)| (*node_id, ScoreBits::from(*score)))
                .collect(),
        }
    }
}

/// `f32` wrapper with stable equality/hash semantics for snapshot structs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScoreBits(u32);

impl ScoreBits {
    /// Return the score as `f32`.
    #[must_use]
    pub fn get(self) -> f32 {
        f32::from_bits(self.0)
    }
}

impl From<f32> for ScoreBits {
    fn from(value: f32) -> Self {
        Self(value.to_bits())
    }
}

pub(crate) fn render_search_rows(label: &str, rows: &SearchRowsSummary, out: &mut Vec<String>) {
    if rows.rows.is_empty() {
        out.push(format!("{label}.rows=[]"));
        return;
    }
    out.push(format!("{label}.rows"));
    for (rank, (node_id, score)) in rows.rows.iter().enumerate() {
        out.push(format!(
            "  [{rank}] {}\t{}",
            render_node_id(*node_id),
            format_score(score.get())
        ));
    }
}
