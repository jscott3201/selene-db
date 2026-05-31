//! OPT-5 cost-estimator unit coverage in isolation.
//!
//! Lives in `tests/` (test-harness gated) rather than inline in
//! `src/plan/optimize/cost.rs` because the `dos_guard` interner-budget scan
//! rejects any direct interner-call substring anywhere under `src/`. The
//! estimators are re-exported under the `test-harness` feature for exactly this
//! purpose (mirroring `optimize_summary`).

use selene_core::{IStr, Value, intern};
use selene_gql::plan::optimize::{
    composite_cost, disjunctive_cost, in_list_cost, linear_baseline, typed_index_cost,
};
use selene_gql::{
    CompositeIndexHandle, IndexCatalog, IndexHandle, IndexKey, IndexTarget, Literal, SourceSpan,
    TypedIndexBounds, TypedIndexLookup,
};

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

/// Catalog that injects synthetic statistics. `Value` is not `Hash`/`Eq` (it
/// carries `f64`), so value-keyed stats are association lists matched by
/// `PartialEq`.
#[derive(Default)]
struct StatCatalog {
    total: Option<u64>,
    label_card: Vec<(IStr, u64)>,
    eq_card: Vec<((IStr, IStr, Value), u64)>,
    avg_bucket: Vec<((IStr, IStr), u64)>,
    composite_card: Vec<((IStr, Vec<Value>), u64)>,
    composite_avg: Vec<(IStr, u64)>,
}

impl StatCatalog {
    fn label(mut self, label: IStr, count: u64) -> Self {
        self.label_card.push((label, count));
        self
    }
    fn eq(mut self, label: IStr, property: IStr, value: Value, count: u64) -> Self {
        self.eq_card.push(((label, property, value), count));
        self
    }
    fn avg(mut self, label: IStr, property: IStr, count: u64) -> Self {
        self.avg_bucket.push(((label, property), count));
        self
    }
    fn composite(mut self, label: IStr, keys: Vec<Value>, count: u64) -> Self {
        self.composite_card.push(((label, keys), count));
        self
    }
    fn composite_avg(mut self, label: IStr, count: u64) -> Self {
        self.composite_avg.push((label, count));
        self
    }
}

impl IndexCatalog for StatCatalog {
    fn typed_index(&self, _t: IndexTarget, _l: IStr, _p: IStr) -> Option<TypedIndexLookup> {
        None
    }
    fn label_index(&self, _t: IndexTarget, _l: IStr) -> Option<IndexHandle> {
        None
    }
    fn composite_index(
        &self,
        _t: IndexTarget,
        _l: IStr,
        _p: &[IStr],
    ) -> Option<CompositeIndexHandle> {
        None
    }
    fn total_rows(&self, _t: IndexTarget) -> Option<u64> {
        self.total
    }
    fn label_cardinality(&self, _t: IndexTarget, label: IStr) -> Option<u64> {
        self.label_card
            .iter()
            .find(|(l, _)| *l == label)
            .map(|(_, c)| *c)
    }
    fn equality_cardinality(
        &self,
        _t: IndexTarget,
        label: IStr,
        property: IStr,
        value: &Value,
    ) -> Option<u64> {
        self.eq_card
            .iter()
            .find(|((l, p, v), _)| *l == label && *p == property && v == value)
            .map(|(_, c)| *c)
    }
    fn range_cardinality(
        &self,
        _t: IndexTarget,
        _label: IStr,
        _property: IStr,
        _range: (std::ops::Bound<Value>, std::ops::Bound<Value>),
    ) -> Option<u64> {
        None
    }
    fn typed_avg_bucket(&self, _t: IndexTarget, label: IStr, property: IStr) -> Option<u64> {
        self.avg_bucket
            .iter()
            .find(|((l, p), _)| *l == label && *p == property)
            .map(|(_, c)| *c)
    }
    fn composite_cardinality(
        &self,
        _t: IndexTarget,
        label: IStr,
        _props: &[IStr],
        keys: &[Value],
    ) -> Option<u64> {
        self.composite_card
            .iter()
            .find(|((l, k), _)| *l == label && k.as_slice() == keys)
            .map(|(_, c)| *c)
    }
    fn composite_avg_bucket(&self, _t: IndexTarget, label: IStr, _props: &[IStr]) -> Option<u64> {
        self.composite_avg
            .iter()
            .find(|(l, _)| *l == label)
            .map(|(_, c)| *c)
    }
}

fn int_lit(v: i64) -> IndexKey {
    IndexKey::Literal(Literal::Integer(v, SourceSpan::new(0, 1)))
}

fn param(name: &str) -> IndexKey {
    IndexKey::Parameter {
        name: istr(name),
        declared_type: None,
        span: SourceSpan::new(0, 1),
    }
}

#[test]
fn linear_baseline_prefers_label_then_total() {
    let person = istr("Person");
    let cat = StatCatalog {
        total: Some(1000),
        ..Default::default()
    }
    .label(person, 42);
    assert_eq!(
        linear_baseline(&cat, IndexTarget::Node, person),
        Some(42),
        "label cardinality wins when present"
    );
    let unknown = istr("Unknown");
    assert_eq!(
        linear_baseline(&cat, IndexTarget::Node, unknown),
        Some(1000),
        "absent label → falls back to total rows"
    );
}

#[test]
fn linear_baseline_none_without_any_stat() {
    let cat = StatCatalog::default();
    assert_eq!(
        linear_baseline(&cat, IndexTarget::Node, istr("X")),
        None,
        "no stats → None → caller keeps structural decision"
    );
}

#[test]
fn equality_literal_is_exact() {
    let (person, age) = (istr("PersonEq"), istr("age"));
    let cat = StatCatalog::default().eq(person, age, Value::Int(30), 7);
    let bounds = TypedIndexBounds::Equality(int_lit(30));
    assert_eq!(
        typed_index_cost(&cat, IndexTarget::Node, person, age, &bounds),
        Some(7)
    );
}

#[test]
fn equality_parameter_uses_avg_bucket() {
    let (person, age) = (istr("PersonParam"), istr("ageP"));
    let cat = StatCatalog::default().avg(person, age, 5);
    let bounds = TypedIndexBounds::Equality(param("p"));
    assert_eq!(
        typed_index_cost(&cat, IndexTarget::Node, person, age, &bounds),
        Some(5)
    );
}

#[test]
fn none_propagates_when_stat_absent() {
    let cat = StatCatalog::default();
    let (person, age) = (istr("PersonNone"), istr("ageNone"));
    let bounds = TypedIndexBounds::Equality(int_lit(30));
    assert_eq!(
        typed_index_cost(&cat, IndexTarget::Node, person, age, &bounds),
        None
    );
}

#[test]
fn in_list_sums_literal_equalities() {
    let (person, age) = (istr("PersonIn"), istr("ageIn"));
    let cat =
        StatCatalog::default()
            .eq(person, age, Value::Int(1), 2)
            .eq(person, age, Value::Int(2), 3);
    let keys = vec![int_lit(1), int_lit(2)];
    assert_eq!(
        in_list_cost(&cat, IndexTarget::Node, person, age, &keys),
        Some(5)
    );
}

#[test]
fn in_list_declines_when_one_element_missing() {
    let (person, age) = (istr("PersonIn2"), istr("ageIn2"));
    // Value::Int(2) has no stat → None propagates.
    let cat = StatCatalog::default().eq(person, age, Value::Int(1), 2);
    let keys = vec![int_lit(1), int_lit(2)];
    assert_eq!(
        in_list_cost(&cat, IndexTarget::Node, person, age, &keys),
        None
    );
}

#[test]
fn composite_literal_is_exact() {
    let (person, a, b) = (istr("PersonComp"), istr("a"), istr("b"));
    let cat = StatCatalog::default().composite(person, vec![Value::Int(1), Value::Int(2)], 9);
    let keys = vec![int_lit(1), int_lit(2)];
    assert_eq!(
        composite_cost(&cat, IndexTarget::Node, person, &[a, b], &keys),
        Some(9)
    );
}

#[test]
fn composite_parameter_uses_avg_bucket() {
    let (person, a, b) = (istr("PersonComp2"), istr("a2"), istr("b2"));
    let cat = StatCatalog::default().composite_avg(person, 4);
    let keys = vec![int_lit(1), param("p")];
    assert_eq!(
        composite_cost(&cat, IndexTarget::Node, person, &[a, b], &keys),
        Some(4)
    );
}

#[test]
fn disjunctive_sums_branch_cardinalities() {
    let (a, b) = (istr("LabelA"), istr("LabelB"));
    let cat = StatCatalog::default().label(a, 10).label(b, 20);
    assert_eq!(disjunctive_cost(&cat, IndexTarget::Node, &[a, b]), Some(30));
}

#[test]
fn disjunctive_declines_when_a_branch_lacks_a_stat() {
    let (a, b) = (istr("LabelDA"), istr("LabelDB"));
    // Only `a` has a label cardinality; `b` is absent → None propagates.
    let cat = StatCatalog::default().label(a, 10);
    assert_eq!(disjunctive_cost(&cat, IndexTarget::Node, &[a, b]), None);
}
