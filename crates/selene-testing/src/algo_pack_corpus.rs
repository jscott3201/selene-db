//! Mirror corpus for `selene-algorithms-pack` procedure invocations.

/// Stable category tag for algorithms-pack corpus entries.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum AlgoPackCorpusCategory {
    /// Projection-catalog procedure coverage.
    Projection,
    /// Algorithm adapter procedure coverage.
    Algorithm,
}

impl AlgoPackCorpusCategory {
    /// All declared algorithms-pack corpus categories.
    pub const ALL: &'static [Self] = &[Self::Projection, Self::Algorithm];

    /// Stable category label used by drift tests to anchor exhaustive matching.
    #[must_use]
    pub const fn stable_label(self) -> &'static str {
        match self {
            Self::Projection => "Projection",
            Self::Algorithm => "Algorithm",
        }
    }
}

const _ASSERT_CATEGORY_ALL_MATCHES_VARIANT_COUNT: () = {
    assert!(AlgoPackCorpusCategory::ALL.len() == 2);
};

/// Procedure invocation mirrored by the algorithms-pack corpus.
#[derive(Clone, Debug, PartialEq)]
pub enum AlgoPackInvocation {
    /// `algo.projection_build`.
    ProjectionBuild {
        /// Projection name.
        name: &'static str,
        /// Node label filter.
        node_labels: &'static [&'static str],
        /// Edge label filter.
        edge_labels: &'static [&'static str],
        /// Optional edge-weight property.
        weight_property: Option<&'static str>,
    },
    /// `algo.projection_get`.
    ProjectionGet {
        /// Projection name.
        name: &'static str,
    },
    /// `algo.projection_drop`.
    ProjectionDrop {
        /// Projection name.
        name: &'static str,
    },
    /// `algo.projection_list`.
    ProjectionList,
    /// `algo.pagerank`.
    Pagerank {
        /// Projection name.
        projection_name: &'static str,
        /// Nullable damping argument.
        damping: Option<f64>,
        /// Nullable max-iteration argument.
        max_iterations: Option<usize>,
        /// Nullable tolerance argument.
        tolerance: Option<f64>,
    },
    /// `algo.betweenness`.
    Betweenness {
        /// Projection name.
        projection_name: &'static str,
        /// Nullable sample-size argument.
        sample_size: Option<usize>,
    },
    /// `algo.label_propagation`.
    LabelPropagation {
        /// Projection name.
        projection_name: &'static str,
        /// Nullable max-iteration argument.
        max_iter: Option<usize>,
    },
    /// `algo.louvain`.
    Louvain {
        /// Projection name.
        projection_name: &'static str,
        /// Nullable max-iteration argument.
        max_iter: Option<usize>,
    },
    /// `algo.triangle_count`.
    TriangleCount {
        /// Projection name.
        projection_name: &'static str,
    },
    /// `algo.wcc`.
    Wcc {
        /// Projection name.
        projection_name: &'static str,
    },
    /// `algo.scc`.
    Scc {
        /// Projection name.
        projection_name: &'static str,
    },
    /// `algo.wcc_count`.
    WccCount {
        /// Projection name.
        projection_name: &'static str,
    },
    /// `algo.scc_count`.
    SccCount {
        /// Projection name.
        projection_name: &'static str,
    },
    /// `algo.topological_sort`.
    TopologicalSort {
        /// Projection name.
        projection_name: &'static str,
    },
    /// `algo.articulation_points`.
    ArticulationPoints {
        /// Projection name.
        projection_name: &'static str,
    },
    /// `algo.bridges`.
    Bridges {
        /// Projection name.
        projection_name: &'static str,
    },
    /// `algo.dijkstra`.
    Dijkstra {
        /// Projection name.
        projection_name: &'static str,
    },
    /// `algo.sssp`.
    Sssp {
        /// Projection name.
        projection_name: &'static str,
    },
    /// `algo.apsp`.
    Apsp {
        /// Projection name.
        projection_name: &'static str,
        /// Required all-pairs bound.
        max_nodes: usize,
    },
}

impl AlgoPackInvocation {
    /// Return the canonical procedure name path for this invocation.
    #[must_use]
    pub const fn procedure_name(&self) -> &'static [&'static str] {
        match self {
            Self::ProjectionBuild { .. } => &["algo", "projection_build"],
            Self::ProjectionGet { .. } => &["algo", "projection_get"],
            Self::ProjectionDrop { .. } => &["algo", "projection_drop"],
            Self::ProjectionList => &["algo", "projection_list"],
            Self::Pagerank { .. } => &["algo", "pagerank"],
            Self::Betweenness { .. } => &["algo", "betweenness"],
            Self::LabelPropagation { .. } => &["algo", "label_propagation"],
            Self::Louvain { .. } => &["algo", "louvain"],
            Self::TriangleCount { .. } => &["algo", "triangle_count"],
            Self::Wcc { .. } => &["algo", "wcc"],
            Self::Scc { .. } => &["algo", "scc"],
            Self::WccCount { .. } => &["algo", "wcc_count"],
            Self::SccCount { .. } => &["algo", "scc_count"],
            Self::TopologicalSort { .. } => &["algo", "topological_sort"],
            Self::ArticulationPoints { .. } => &["algo", "articulation_points"],
            Self::Bridges { .. } => &["algo", "bridges"],
            Self::Dijkstra { .. } => &["algo", "dijkstra"],
            Self::Sssp { .. } => &["algo", "sssp"],
            Self::Apsp { .. } => &["algo", "apsp"],
        }
    }

    /// Render a deterministic GQL `CALL` string for snapshot testing.
    #[must_use]
    pub fn render_call(&self) -> String {
        match self {
            Self::ProjectionBuild {
                name,
                node_labels,
                edge_labels,
                weight_property,
            } => format!(
                "CALL algo.projection_build({}, {}, {}, {})",
                quoted(name),
                label_list(node_labels),
                label_list(edge_labels),
                nullable_string(*weight_property)
            ),
            Self::ProjectionGet { name } => {
                format!("CALL algo.projection_get({})", quoted(name))
            }
            Self::ProjectionDrop { name } => {
                format!("CALL algo.projection_drop({})", quoted(name))
            }
            Self::ProjectionList => "CALL algo.projection_list()".to_string(),
            Self::Pagerank {
                projection_name,
                damping,
                max_iterations,
                tolerance,
            } => format!(
                "CALL algo.pagerank({}, {}, {}, {})",
                quoted(projection_name),
                nullable_f64(*damping),
                nullable_usize(*max_iterations),
                nullable_f64(*tolerance)
            ),
            Self::Betweenness {
                projection_name,
                sample_size,
            } => format!(
                "CALL algo.betweenness({}, {})",
                quoted(projection_name),
                nullable_usize(*sample_size)
            ),
            Self::LabelPropagation {
                projection_name,
                max_iter,
            } => format!(
                "CALL algo.label_propagation({}, {})",
                quoted(projection_name),
                nullable_usize(*max_iter)
            ),
            Self::Louvain {
                projection_name,
                max_iter,
            } => format!(
                "CALL algo.louvain({}, {})",
                quoted(projection_name),
                nullable_usize(*max_iter)
            ),
            Self::TriangleCount { projection_name } => {
                format!(
                    "CALL algo.triangle_count({}, NULL)",
                    quoted(projection_name)
                )
            }
            Self::Wcc { projection_name } => {
                format!("CALL algo.wcc({})", quoted(projection_name))
            }
            Self::Scc { projection_name } => {
                format!("CALL algo.scc({})", quoted(projection_name))
            }
            Self::WccCount { projection_name } => {
                format!("CALL algo.wcc_count({})", quoted(projection_name))
            }
            Self::SccCount { projection_name } => {
                format!("CALL algo.scc_count({})", quoted(projection_name))
            }
            Self::TopologicalSort { projection_name } => {
                format!("CALL algo.topological_sort({})", quoted(projection_name))
            }
            Self::ArticulationPoints { projection_name } => {
                format!("CALL algo.articulation_points({})", quoted(projection_name))
            }
            Self::Bridges { projection_name } => {
                format!("CALL algo.bridges({})", quoted(projection_name))
            }
            Self::Dijkstra { projection_name } => {
                format!(
                    "MATCH (source), (target) CALL algo.dijkstra({}, source, target) YIELD cost, path, length",
                    quoted(projection_name)
                )
            }
            Self::Sssp { projection_name } => {
                format!(
                    "MATCH (source) CALL algo.sssp({}, source) YIELD target_node, cost",
                    quoted(projection_name)
                )
            }
            Self::Apsp {
                projection_name,
                max_nodes,
            } => {
                format!(
                    "CALL algo.apsp({}, {max_nodes}, NULL)",
                    quoted(projection_name)
                )
            }
        }
    }
}

/// One algorithms-pack corpus entry.
#[derive(Clone, Debug, PartialEq)]
pub struct AlgoPackCorpusEntry {
    /// Stable entry name.
    pub name: &'static str,
    /// Category for coverage accounting.
    pub category: AlgoPackCorpusCategory,
    /// Mirrored invocation.
    pub invocation: AlgoPackInvocation,
}

impl AlgoPackCorpusEntry {
    /// Render this entry to a deterministic single-line snapshot row.
    #[must_use]
    pub fn render(&self) -> String {
        format!(
            "{} [{:?}] {}\n",
            self.name,
            self.category,
            self.invocation.render_call()
        )
    }
}

/// Corpus mirror used by algorithms-pack tests.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AlgoPackCorpus {
    entries: Vec<AlgoPackCorpusEntry>,
}

impl AlgoPackCorpus {
    /// Construct the B1 seed corpus.
    #[must_use]
    pub fn b1_seed() -> Self {
        Self {
            entries: vec![
                AlgoPackCorpusEntry {
                    name: "projection_build_all",
                    category: AlgoPackCorpusCategory::Projection,
                    invocation: AlgoPackInvocation::ProjectionBuild {
                        name: "p",
                        node_labels: &[],
                        edge_labels: &[],
                        weight_property: None,
                    },
                },
                AlgoPackCorpusEntry {
                    name: "projection_get",
                    category: AlgoPackCorpusCategory::Projection,
                    invocation: AlgoPackInvocation::ProjectionGet { name: "p" },
                },
                AlgoPackCorpusEntry {
                    name: "projection_drop",
                    category: AlgoPackCorpusCategory::Projection,
                    invocation: AlgoPackInvocation::ProjectionDrop { name: "p" },
                },
                AlgoPackCorpusEntry {
                    name: "projection_list",
                    category: AlgoPackCorpusCategory::Projection,
                    invocation: AlgoPackInvocation::ProjectionList,
                },
                AlgoPackCorpusEntry {
                    name: "pagerank_defaults",
                    category: AlgoPackCorpusCategory::Algorithm,
                    invocation: AlgoPackInvocation::Pagerank {
                        projection_name: "p",
                        damping: None,
                        max_iterations: None,
                        tolerance: None,
                    },
                },
            ],
        }
    }

    /// Construct the B2 corpus including structural adapters.
    #[must_use]
    pub fn b2_seed() -> Self {
        let mut corpus = Self::b1_seed();
        corpus.entries.extend([
            AlgoPackCorpusEntry {
                name: "wcc",
                category: AlgoPackCorpusCategory::Algorithm,
                invocation: AlgoPackInvocation::Wcc {
                    projection_name: "p",
                },
            },
            AlgoPackCorpusEntry {
                name: "scc",
                category: AlgoPackCorpusCategory::Algorithm,
                invocation: AlgoPackInvocation::Scc {
                    projection_name: "p",
                },
            },
            AlgoPackCorpusEntry {
                name: "wcc_count",
                category: AlgoPackCorpusCategory::Algorithm,
                invocation: AlgoPackInvocation::WccCount {
                    projection_name: "p",
                },
            },
            AlgoPackCorpusEntry {
                name: "scc_count",
                category: AlgoPackCorpusCategory::Algorithm,
                invocation: AlgoPackInvocation::SccCount {
                    projection_name: "p",
                },
            },
            AlgoPackCorpusEntry {
                name: "topological_sort",
                category: AlgoPackCorpusCategory::Algorithm,
                invocation: AlgoPackInvocation::TopologicalSort {
                    projection_name: "p",
                },
            },
            AlgoPackCorpusEntry {
                name: "articulation_points",
                category: AlgoPackCorpusCategory::Algorithm,
                invocation: AlgoPackInvocation::ArticulationPoints {
                    projection_name: "p",
                },
            },
            AlgoPackCorpusEntry {
                name: "bridges",
                category: AlgoPackCorpusCategory::Algorithm,
                invocation: AlgoPackInvocation::Bridges {
                    projection_name: "p",
                },
            },
        ]);
        corpus
    }

    /// Construct the B3 corpus including pathfinding adapters.
    #[must_use]
    pub fn b3_seed() -> Self {
        let mut corpus = Self::b2_seed();
        corpus.entries.extend([
            AlgoPackCorpusEntry {
                name: "dijkstra",
                category: AlgoPackCorpusCategory::Algorithm,
                invocation: AlgoPackInvocation::Dijkstra {
                    projection_name: "p",
                },
            },
            AlgoPackCorpusEntry {
                name: "sssp",
                category: AlgoPackCorpusCategory::Algorithm,
                invocation: AlgoPackInvocation::Sssp {
                    projection_name: "p",
                },
            },
            AlgoPackCorpusEntry {
                name: "apsp",
                category: AlgoPackCorpusCategory::Algorithm,
                invocation: AlgoPackInvocation::Apsp {
                    projection_name: "p",
                    max_nodes: 100,
                },
            },
        ]);
        corpus
    }

    /// Construct the B4 corpus including betweenness centrality.
    #[must_use]
    pub fn b4_seed() -> Self {
        let mut corpus = Self::b3_seed();
        corpus.entries.insert(
            5,
            AlgoPackCorpusEntry {
                name: "betweenness_defaults",
                category: AlgoPackCorpusCategory::Algorithm,
                invocation: AlgoPackInvocation::Betweenness {
                    projection_name: "p",
                    sample_size: None,
                },
            },
        );
        corpus
    }

    /// Construct the B5 corpus including community adapters.
    #[must_use]
    pub fn b5_seed() -> Self {
        let mut corpus = Self::b4_seed();
        corpus.entries.splice(
            6..6,
            [
                AlgoPackCorpusEntry {
                    name: "label_propagation_defaults",
                    category: AlgoPackCorpusCategory::Algorithm,
                    invocation: AlgoPackInvocation::LabelPropagation {
                        projection_name: "p",
                        max_iter: None,
                    },
                },
                AlgoPackCorpusEntry {
                    name: "louvain_defaults",
                    category: AlgoPackCorpusCategory::Algorithm,
                    invocation: AlgoPackInvocation::Louvain {
                        projection_name: "p",
                        max_iter: None,
                    },
                },
                AlgoPackCorpusEntry {
                    name: "triangle_count",
                    category: AlgoPackCorpusCategory::Algorithm,
                    invocation: AlgoPackInvocation::TriangleCount {
                        projection_name: "p",
                    },
                },
            ],
        );
        corpus
    }

    /// Borrow corpus entries in deterministic order.
    #[must_use]
    pub fn entries(&self) -> &[AlgoPackCorpusEntry] {
        &self.entries
    }

    /// Render the corpus to a deterministic snapshot string.
    #[must_use]
    pub fn render(&self) -> String {
        self.entries
            .iter()
            .map(AlgoPackCorpusEntry::render)
            .collect()
    }
}

fn quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "\\'"))
}

fn label_list(labels: &[&str]) -> String {
    if labels.is_empty() {
        return "NULL".to_string();
    }
    let rendered = labels.iter().map(|label| quoted(label)).collect::<Vec<_>>();
    format!("[{}]", rendered.join(", "))
}

fn nullable_string(value: Option<&str>) -> String {
    value.map_or_else(|| "NULL".to_string(), quoted)
}

fn nullable_f64(value: Option<f64>) -> String {
    value.map_or_else(|| "NULL".to_string(), |value| value.to_string())
}

fn nullable_usize(value: Option<usize>) -> String {
    value.map_or_else(|| "NULL".to_string(), |value| value.to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{AlgoPackCorpus, AlgoPackCorpusCategory, AlgoPackInvocation};

    #[test]
    fn procedure_name_returns_static_slice_for_pagerank_variant() {
        let invocation = AlgoPackInvocation::Pagerank {
            projection_name: "p",
            damping: None,
            max_iterations: None,
            tolerance: None,
        };

        assert_eq!(invocation.procedure_name(), &["algo", "pagerank"]);
    }

    #[test]
    fn procedure_name_returns_static_slice_for_betweenness_variant() {
        let invocation = AlgoPackInvocation::Betweenness {
            projection_name: "p",
            sample_size: None,
        };

        assert_eq!(invocation.procedure_name(), &["algo", "betweenness"]);
    }

    #[test]
    fn procedure_name_returns_static_slice_for_louvain_variant() {
        let invocation = AlgoPackInvocation::Louvain {
            projection_name: "p",
            max_iter: None,
        };

        assert_eq!(invocation.procedure_name(), &["algo", "louvain"]);
    }

    #[test]
    fn procedure_name_covers_all_19_variants() {
        let invocations = [
            AlgoPackInvocation::ProjectionBuild {
                name: "p",
                node_labels: &[],
                edge_labels: &[],
                weight_property: None,
            },
            AlgoPackInvocation::ProjectionGet { name: "p" },
            AlgoPackInvocation::ProjectionDrop { name: "p" },
            AlgoPackInvocation::ProjectionList,
            AlgoPackInvocation::Pagerank {
                projection_name: "p",
                damping: None,
                max_iterations: None,
                tolerance: None,
            },
            AlgoPackInvocation::Betweenness {
                projection_name: "p",
                sample_size: None,
            },
            AlgoPackInvocation::LabelPropagation {
                projection_name: "p",
                max_iter: None,
            },
            AlgoPackInvocation::Louvain {
                projection_name: "p",
                max_iter: None,
            },
            AlgoPackInvocation::TriangleCount {
                projection_name: "p",
            },
            AlgoPackInvocation::Wcc {
                projection_name: "p",
            },
            AlgoPackInvocation::Scc {
                projection_name: "p",
            },
            AlgoPackInvocation::WccCount {
                projection_name: "p",
            },
            AlgoPackInvocation::SccCount {
                projection_name: "p",
            },
            AlgoPackInvocation::TopologicalSort {
                projection_name: "p",
            },
            AlgoPackInvocation::ArticulationPoints {
                projection_name: "p",
            },
            AlgoPackInvocation::Bridges {
                projection_name: "p",
            },
            AlgoPackInvocation::Dijkstra {
                projection_name: "p",
            },
            AlgoPackInvocation::Sssp {
                projection_name: "p",
            },
            AlgoPackInvocation::Apsp {
                projection_name: "p",
                max_nodes: 100,
            },
        ];

        let mut names = BTreeSet::new();
        for invocation in invocations {
            let name = invocation.procedure_name();
            assert_eq!(name.len(), 2);
            assert_eq!(name[0], "algo");
            assert!(names.insert(name));
        }
        assert_eq!(names.len(), 19);
    }

    #[test]
    fn category_all_matches_exhaustive_anchor() {
        let declared = AlgoPackCorpusCategory::ALL
            .iter()
            .map(|category| category.stable_label())
            .collect::<BTreeSet<_>>();

        assert_eq!(
            declared,
            ["Projection", "Algorithm"]
                .into_iter()
                .collect::<BTreeSet<_>>()
        );
    }

    #[test]
    fn entry_render_matches_corpus_render_line() {
        let corpus = AlgoPackCorpus::b5_seed();
        let rendered_entries = corpus
            .entries()
            .iter()
            .map(super::AlgoPackCorpusEntry::render)
            .collect::<String>();

        assert_eq!(corpus.render(), rendered_entries);
    }
}
