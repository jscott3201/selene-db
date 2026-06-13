//! Generated seeded-scan oracle checks.

use super::{execute_on_graph, graph_with_people, ints};

#[test]
fn generated_seeded_range_and_in_queries_match_reference() {
    let groups = ["g0", "g1", "g2", "g0", "g2", "g1", "g0", "g2"];
    for seed in 0..8 {
        let ages = generated_ages(seed);
        let graph = graph_with_people(12_400 + seed as u64, &ages, &groups);
        for threshold in [18, 30, 42, 54] {
            let source = format!(
                "MATCH (a:Person)
                 MATCH (a:Person)
                 WHERE a.age >= {threshold}
                 RETURN a.id AS id
                 ORDER BY id"
            );
            let table = execute_on_graph(&graph, &source).expect("range query executes");
            let expected = ages
                .iter()
                .enumerate()
                .filter_map(|(index, age)| (*age >= threshold).then_some(index as i64))
                .collect::<Vec<_>>();
            assert_eq!(ints(&table, "id"), expected);
        }

        let table = execute_on_graph(
            &graph,
            "MATCH (a:Person)
             MATCH (a:Person)
             WHERE a.grp IN ['g0', 'g2']
             RETURN a.id AS id
             ORDER BY id",
        )
        .expect("bitmap query executes");
        let expected = groups
            .iter()
            .enumerate()
            .filter_map(|(index, group)| matches!(*group, "g0" | "g2").then_some(index as i64))
            .collect::<Vec<_>>();
        assert_eq!(ints(&table, "id"), expected);
    }
}

fn generated_ages(seed: usize) -> Vec<i64> {
    (0..8)
        .map(|index| {
            let index = index as i64;
            let raw = (seed as i64 * 37 + index * 17 + index * index * 3) % 61;
            18 + raw
        })
        .collect()
}
