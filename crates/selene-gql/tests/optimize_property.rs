//! BRIEF-28 optimizer property tests.

use proptest::test_runner::TestRunner;
use selene_gql::{EmptyProcedureRegistry, analyze, optimize, parse, plan};

#[test]
fn representative_plans_are_idempotent_under_proptest() {
    let sources = vec![
        "RETURN 1 + 2 AS x",
        "MATCH (n) FILTER n.a > 1 RETURN n",
        "MATCH (n) FILTER n.a > 1 LIMIT 10 FILTER n.b < 5 RETURN n",
        "MATCH (n)-[e]->(m) FILTER e.weight > 1 RETURN n, m",
        "MATCH (a)-[e]->(b), (c)-[f]->(d) FILTER f.weight > 1 RETURN a",
        "MATCH (n) RETURN n ORDER BY n.age LIMIT 10",
        "MATCH (n) RETURN n ORDER BY n.age OFFSET 5",
        "INSERT (:Person {age: 1 + 2})",
        "CREATE NODE TYPE :Person (age :: INT DEFAULT 0)",
    ];

    let mut runner = TestRunner::default();
    runner
        .run(&proptest::sample::select(sources), |source| {
            let statement = parse(source).expect("test input parses");
            let analyzed =
                analyze(statement, &EmptyProcedureRegistry, None).expect("test input analyzes");
            let plan = plan(&analyzed, &EmptyProcedureRegistry).expect("test input plans");
            let once = optimize(plan, &selene_gql::OptimizeContext::default());
            let twice = optimize(once.clone(), &selene_gql::OptimizeContext::default());
            proptest::prop_assert_eq!(format!("{once:?}"), format!("{twice:?}"));
            Ok(())
        })
        .expect("optimizer is idempotent for representative plans");
}
