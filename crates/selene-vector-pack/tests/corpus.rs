//! Vector-pack mirror corpus tests.

use selene_core::{IStr, intern};
use selene_gql::{ProcedureMutability, ProcedureRegistry, ProcedureTier};
use selene_vector_pack::{VECTOR_PROCEDURE_NAMES, VectorPack};

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

#[test]
fn registry_exposes_three_graph_tier_vector_procedures() {
    let pack = VectorPack::new();
    let registry = pack
        .registry_with_builtins()
        .expect("vector pack registers cleanly");
    let mut graph_tier_count = 0;

    for name in VECTOR_PROCEDURE_NAMES {
        let interned = name.iter().map(|segment| istr(segment)).collect::<Vec<_>>();
        let metadata = registry.lookup(&interned).expect("procedure registered");
        if metadata.tier == ProcedureTier::Graph {
            graph_tier_count += 1;
            assert_eq!(metadata.mutability, ProcedureMutability::Read);
        }
    }

    assert_eq!(graph_tier_count, 3);
}
