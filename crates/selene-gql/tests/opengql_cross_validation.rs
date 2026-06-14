//! Cross-validation against vendored opengql/grammar samples.

use std::{fs, path::Path};

use selene_gql::{ParserError, parse};

const SYNTAX_GAP_ALLOWED: &[&str] = &[
    "create_closed_graph_from_graph_type_(double_colon).gql",
    "create_closed_graph_from_graph_type_(lexical).gql",
    "create_closed_graph_from_nested_graph_type_(double_colon).gql",
    "create_graph.gql",
    "create_schema.gql",
    "match_with_exists_predicate_(nested_match_statement).gql",
    "session_set_graph_to_current_graph.gql",
    "session_set_graph_to_current_property_graph.gql",
];

#[test]
fn opengql_samples_return_structured_results() {
    let samples = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../selene-testing/opengql/samples")
        .canonicalize()
        .expect("sample directory exists");
    let mut files = fs::read_dir(&samples)
        .expect("sample directory readable")
        .map(|entry| entry.expect("sample entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "gql"))
        .collect::<Vec<_>>();
    files.sort();
    assert!(!files.is_empty(), "opengql samples must be vendored");

    for file in files {
        let source = fs::read_to_string(&file).expect("sample is utf-8");
        let file_name = file
            .file_name()
            .and_then(|name| name.to_str())
            .expect("utf-8 sample file name");
        for chunk in chunks(&source) {
            match parse(chunk) {
                Ok(_) => {}
                Err(
                    ParserError::UnsupportedFeature { .. } | ParserError::NotImplemented { .. },
                ) => {}
                Err(ParserError::SyntaxError { .. }) => assert!(
                    SYNTAX_GAP_ALLOWED.contains(&file_name),
                    "{file_name}: unexpected syntax drift for {chunk:?}"
                ),
                Err(other) => panic!("{file_name}: unexpected parser error {other:?}"),
            }
        }
    }
}

fn chunks(source: &str) -> impl Iterator<Item = &str> {
    source
        .split("\n\n")
        .map(str::trim)
        .filter(|chunk| !chunk.is_empty())
}
