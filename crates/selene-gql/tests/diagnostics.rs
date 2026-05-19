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
    let error = parse_with_source(Arc::<str>::from("RETURN 1 OTHERWISE RETURN 2"), "query.gql")
        .expect_err("OTHERWISE is unclaimed");
    let ParserError::UnsupportedFeature { feature_id, .. } = error.error() else {
        panic!("expected UnsupportedFeature");
    };
    assert_eq!(*feature_id, FeatureId::GQ09);

    let rendered = render(&error);
    assert!(rendered.contains("GQ09"));
}

#[test]
fn diagnostic_report_wraps_interner_budget_exceeded() {
    let error = ParserError::InternerBudgetExceeded {
        limit: 8_192,
        span: SourceSpan::new(7, 5),
    };
    let report = DiagnosticReport::new(error, Arc::<str>::from("RETURN over_budget"), "query.gql");

    let rendered = render(&report);
    assert!(rendered.contains("5GQL1"));
    assert!(rendered.contains("interner-admission budget exceeded"));
    assert!(rendered.contains("8192 distinct new interner admissions"));
}

fn render(report: &DiagnosticReport) -> String {
    let mut rendered = String::new();
    NarratableReportHandler::new()
        .render_report(&mut rendered, report)
        .expect("render report");
    rendered
}
