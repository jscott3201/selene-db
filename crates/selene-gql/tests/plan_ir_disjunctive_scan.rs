//! BRIEF-155 Commit 1 — `JoinTree::DisjunctiveScan` IR-shape smoke tests.
//!
//! The variant is rule-emitted (Commit 3 adds `DisjunctiveLabelExpansion`),
//! so until then no planner path produces a `DisjunctiveScan`. These tests
//! hand-construct the variant via the public IR API to confirm the new
//! shape is representable, clones round-trip, and debug-formatting works.

use selene_core::{IStr, intern};
use selene_gql::{
    JoinTree, LabelExpr, NodeOrEdgeScan, ScanAccess, ScanKind, SourceSpan, Vec2OrMore,
};

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

fn linear_node_scan(label: IStr) -> NodeOrEdgeScan {
    NodeOrEdgeScan {
        binding: None,
        hidden_binding: None,
        kind: ScanKind::Node,
        label_predicate: Some(LabelExpr::Single(label)),
        property_predicates: Vec::new(),
        access: ScanAccess::Linear,
        span: SourceSpan::new(0, 0),
    }
}

fn disjunction_anchor(labels: &[IStr]) -> NodeOrEdgeScan {
    let parts: Vec<LabelExpr> = labels
        .iter()
        .map(|label| LabelExpr::Single(label.clone()))
        .collect();
    let disjunction = LabelExpr::Disjunction(
        Vec2OrMore::try_from_vec(parts).expect("≥ 2 labels for a disjunction anchor"),
    );
    NodeOrEdgeScan {
        binding: None,
        hidden_binding: None,
        kind: ScanKind::Node,
        label_predicate: Some(disjunction),
        property_predicates: Vec::new(),
        access: ScanAccess::Linear,
        span: SourceSpan::new(0, 0),
    }
}

#[test]
fn disjunctive_scan_construction_two_branches() {
    let a = istr("A");
    let b = istr("B");
    let branches = vec![linear_node_scan(a.clone()), linear_node_scan(b.clone())];
    let scan_anchor = disjunction_anchor(&[a, b]);

    let tree = JoinTree::DisjunctiveScan {
        branches: branches.clone(),
        scan_anchor,
    };

    // The variant is constructible with the smallest valid branch count.
    if let JoinTree::DisjunctiveScan {
        branches,
        scan_anchor,
    } = &tree
    {
        assert_eq!(branches.len(), 2);
        assert!(matches!(
            scan_anchor.label_predicate,
            Some(LabelExpr::Disjunction(_))
        ));
        for branch in branches {
            assert!(matches!(branch.label_predicate, Some(LabelExpr::Single(_))));
            assert!(matches!(branch.access, ScanAccess::Linear));
            assert_eq!(branch.kind, ScanKind::Node);
        }
    } else {
        panic!("expected JoinTree::DisjunctiveScan");
    }
}

#[test]
fn disjunctive_scan_clone_and_debug() {
    let labels: Vec<IStr> = ["Module", "Namespace", "Class"]
        .iter()
        .copied()
        .map(istr)
        .collect();
    let branches: Vec<NodeOrEdgeScan> = labels.iter().cloned().map(linear_node_scan).collect();
    let tree = JoinTree::DisjunctiveScan {
        branches,
        scan_anchor: disjunction_anchor(&labels),
    };

    let cloned = tree.clone();
    if let JoinTree::DisjunctiveScan {
        branches: original_branches,
        ..
    } = &tree
        && let JoinTree::DisjunctiveScan {
            branches: cloned_branches,
            ..
        } = &cloned
    {
        assert_eq!(original_branches.len(), cloned_branches.len());
        assert_eq!(cloned_branches.len(), 3);
    } else {
        panic!("clone returned wrong variant");
    }

    // Debug formatting includes the variant name so EXPLAIN / planner
    // diagnostics surface it without bespoke rendering.
    let formatted = format!("{tree:?}");
    assert!(formatted.contains("DisjunctiveScan"));
    assert!(formatted.contains("branches"));
    assert!(formatted.contains("scan_anchor"));
}

#[test]
fn disjunctive_scan_branches_inherit_scan_kind() {
    // Even though the rule only fires on `ScanKind::Node` (F6), the IR shape
    // itself doesn't restrict the kind — guard against accidental coupling.
    let label = istr("Foo");
    let mut branch = linear_node_scan(label.clone());
    branch.kind = ScanKind::Node;
    let anchor = disjunction_anchor(&[label, istr("Bar")]);

    let tree = JoinTree::DisjunctiveScan {
        branches: vec![branch.clone(), branch],
        scan_anchor: anchor,
    };
    if let JoinTree::DisjunctiveScan { branches, .. } = &tree {
        for branch in branches {
            assert_eq!(branch.kind, ScanKind::Node);
        }
    }
}
