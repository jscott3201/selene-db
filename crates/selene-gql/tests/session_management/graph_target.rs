use super::*;

#[test]
fn set_graph_current_graph_executes_as_single_graph_noop() {
    let graph = graph(7018);
    let mut session = Session::new(&graph);

    assert!(matches!(
        run(&mut session, "SESSION SET GRAPH CURRENT_GRAPH").expect("set graph"),
        StatementOutput::Empty
    ));
    assert!(!session.is_closed());
}

#[test]
fn set_property_graph_current_property_graph_executes_as_single_graph_noop() {
    let graph = graph(7019);
    let mut session = Session::new(&graph);

    assert!(matches!(
        run(
            &mut session,
            "SESSION SET PROPERTY GRAPH CURRENT_PROPERTY_GRAPH"
        )
        .expect("set property graph"),
        StatementOutput::Empty
    ));
    assert!(!session.is_closed());
}

#[test]
fn set_graph_current_targets_parse_to_distinct_ast_targets() {
    let statement = parse("SESSION SET GRAPH CURRENT_GRAPH").expect("parse current graph");
    assert!(matches!(
        statement,
        Statement::SessionSetGraph {
            target: SessionSetGraphTarget::CurrentGraph,
            ..
        }
    ));

    let statement = parse("SESSION SET PROPERTY GRAPH CURRENT_PROPERTY_GRAPH")
        .expect("parse current property graph");
    assert!(matches!(
        statement,
        Statement::SessionSetGraph {
            target: SessionSetGraphTarget::CurrentPropertyGraph,
            ..
        }
    ));
}

#[test]
fn set_graph_current_targets_do_not_stamp_graph_parameter_feature() {
    assert!(walked_features("SESSION SET GRAPH CURRENT_GRAPH").is_empty());
    assert!(walked_features("SESSION SET PROPERTY GRAPH CURRENT_PROPERTY_GRAPH").is_empty());
}
