//! Summary-only statement outcomes.

use crate::GqlStatus;

/// Summary returned by [`Session::execute`](crate::Session::execute).
///
/// M02-PR01 does not expose row values. A row-bearing statement reports only
/// cardinality, while a committed write reports its change count and optional
/// returned-row count. A database-catalog statement reports the ISO
/// completion condition of its omitted result.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ExecutionOutcome {
    /// The statement completed without rows or committed graph changes.
    Empty,
    /// A database-catalog statement completed with an omitted result
    /// (ISO/IEC 39075:2024 §4.9.3, §12.1 GR2).
    ///
    /// `status` is `00001` for every successful `CREATE/DROP SCHEMA` and
    /// `CREATE/DROP GRAPH`, including the `IF [NOT] EXISTS` no-ops that §12.2,
    /// §12.3, and §12.4 define without a warning, and `01G03` for `DROP GRAPH
    /// IF EXISTS` on an absent graph (§12.5 GR1). A no-op publishes no catalog
    /// state.
    OmittedResult {
        /// Completion condition of the statement.
        status: GqlStatus,
    },
    /// The statement produced a row-bearing result.
    Rows {
        /// Number of rows produced.
        row_count: usize,
    },
    /// The statement committed graph changes.
    Written(WriteSummary),
}

/// Summary of an auto-committed write.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WriteSummary {
    change_count: usize,
    returned_row_count: Option<usize>,
}

impl WriteSummary {
    /// Construct a write summary from committed change and returned-row counts.
    #[must_use]
    pub const fn new(change_count: usize, returned_row_count: Option<usize>) -> Self {
        Self {
            change_count,
            returned_row_count,
        }
    }

    /// Return the number of committed graph changes.
    #[must_use]
    pub const fn change_count(self) -> usize {
        self.change_count
    }

    /// Return the row count for a write with `RETURN`, when present.
    #[must_use]
    pub const fn returned_row_count(self) -> Option<usize> {
        self.returned_row_count
    }
}

impl ExecutionOutcome {
    pub(crate) fn from_engine(output: selene_gql::StatementOutput) -> crate::Result<Self> {
        let summary = match output {
            selene_gql::StatementOutput::Empty => Self::Empty,
            selene_gql::StatementOutput::Rows(rows) => Self::Rows {
                row_count: rows.row_count(),
            },
            selene_gql::StatementOutput::Written(write) => Self::Written(WriteSummary::new(
                write.changes.len(),
                write.rows.as_ref().map(selene_gql::BindingTable::row_count),
            )),
            _ => return Err(crate::Error::unsupported_engine_outcome()),
        };
        Ok(summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lower_empty_output_maps_to_facade_empty_summary() {
        assert_eq!(
            ExecutionOutcome::from_engine(selene_gql::StatementOutput::Empty)
                .expect("known lower outcome maps"),
            ExecutionOutcome::Empty
        );
    }
}
