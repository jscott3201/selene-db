//! Source-backed parser diagnostics.

use std::sync::Arc;

use miette::NarratableReportHandler;
use selene_core::feature_register::FeatureId;
use selene_gql::{DiagnosticReport, ParserError, SourceSpan, parse_with_source};

#[test]
fn parse_with_source_syntax_error_renders_source_highlight() {
    let error = parse_with_source(Arc::<str>::from("RETURN"), "query.gql")
        .expect_err("invalid query should report");
    assert!(matches!(error.error(), ParserError::SyntaxError { .. }));

    let rendered = render(&error);
    assert!(rendered.contains("query.gql"));
    assert!(rendered.contains("highlight"));
}

#[test]
fn parse_with_source_reports_unsupported_feature() {
    let error = parse_with_source(Arc::<str>::from("RETURN n IS TYPED REAL"), "query.gql")
        .expect_err("REAL spelling is unclaimed");
    let ParserError::UnsupportedFeature { feature_id, .. } = error.error() else {
        panic!("expected UnsupportedFeature");
    };
    assert_eq!(*feature_id, FeatureId::GV20);

    let rendered = render(&error);
    assert!(rendered.contains("GV20"));
}

#[test]
fn diagnostic_report_wraps_complexity_limit_exceeded() {
    // Retargeted from the removed interner-budget render test: proves the
    // 5GQL1 PROGRAM_LIMIT_EXCEEDED class still renders for a surviving
    // parser DoS-guard error variant (the #218 bracket-depth guard).
    let error = ParserError::ComplexityLimitExceeded {
        limit: 32,
        span: SourceSpan::new(7, 5),
    };
    let report = DiagnosticReport::new(error, Arc::<str>::from("RETURN over_complex"), "query.gql");

    let rendered = render(&report);
    assert!(rendered.contains("5GQL1"));
    assert!(rendered.contains("parser complexity limit exceeded"));
}

#[test]
fn diagnostic_report_exposes_source_and_label() {
    // PARSE-21: the public `source()` / `named_source()` accessors had no test.
    // Pin that they return the exact source text and the label passed at
    // construction (via miette's NamedSource name).
    let source = "MATCH (n RETURN n";
    let report = parse_with_source(Arc::<str>::from(source), "query.gql")
        .expect_err("invalid query reports");

    assert_eq!(report.source(), source, "source() must echo the input text");

    let named = report.named_source();
    assert_eq!(
        named.name(),
        "query.gql",
        "named_source() carries the label"
    );
    // The wrapped error is also reachable and is the parser error, not a
    // re-wrapped variant.
    assert!(matches!(report.error(), ParserError::SyntaxError { .. }));
}

fn render(report: &DiagnosticReport) -> String {
    let mut rendered = String::new();
    NarratableReportHandler::new()
        .render_report(&mut rendered, report)
        .expect("render report");
    rendered
}
