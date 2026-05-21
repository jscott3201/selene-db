//! Contract tests for `vector.list_indexes`.

use selene_core::{IStr, intern};
use selene_gql::{GqlType, ProcedureRegistry};
use selene_vector_pack::VectorPack;

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

#[test]
fn vector_list_indexes_declares_stable_columns() {
    let pack = VectorPack::new();
    let registry = pack
        .registry_with_builtins()
        .expect("vector pack registers cleanly");
    let name = [istr("vector"), istr("list_indexes")];
    let metadata = registry.lookup(&name).expect("procedure registered");
    let columns = metadata.output_schema.columns;

    assert_eq!(columns.len(), 5);
    assert_eq!(columns[0].name.as_str(), "name");
    assert_eq!(columns[0].ty, GqlType::String);
    assert_eq!(columns[1].name.as_str(), "kind");
    assert_eq!(columns[1].ty, GqlType::String);
    assert_eq!(columns[2].name.as_str(), "dim");
    assert_eq!(columns[2].ty, GqlType::Integer);
    assert_eq!(columns[3].name.as_str(), "metric");
    assert_eq!(columns[3].ty, GqlType::String);
    assert_eq!(columns[4].name.as_str(), "vector_count");
    assert_eq!(columns[4].ty, GqlType::Integer);
}
