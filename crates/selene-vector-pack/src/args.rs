//! Small explicit argument parsers for vector adapters.

use roaring::RoaringBitmap;
use selene_core::{NodeId, Value};
use selene_gql::ProcedureError;
use selene_vector::DistanceMetric;

use crate::error::invalid_argument;

pub(crate) fn expect_arity(
    procedure: &'static str,
    args: &[Value],
    expected: usize,
) -> Result<(), ProcedureError> {
    if args.len() == expected {
        return Ok(());
    }
    Err(invalid_argument(format!(
        "{procedure} expected {expected} arguments, got {}",
        args.len()
    )))
}

pub(crate) fn required_string(
    procedure: &'static str,
    args: &[Value],
    index: usize,
    name: &'static str,
) -> Result<String, ProcedureError> {
    match &args[index] {
        Value::String(value) => Ok(value.as_str().to_owned()),
        Value::ExternalString(value) => Ok(value.to_string()),
        other => Err(invalid_argument(format!(
            "{procedure} expected {name} to be STRING, got {other:?}"
        ))),
    }
}

pub(crate) fn required_f32_list(
    procedure: &'static str,
    args: &[Value],
    index: usize,
    name: &'static str,
) -> Result<Vec<f32>, ProcedureError> {
    let Value::List(values) = &args[index] else {
        return Err(invalid_argument(format!(
            "{procedure} expected {name} to be LIST<FLOAT>, got {:?}",
            args[index]
        )));
    };
    values
        .iter()
        .enumerate()
        .map(|(item_index, value)| numeric_to_f32(procedure, name, item_index, value))
        .collect()
}

pub(crate) fn required_node_ref(
    procedure: &'static str,
    args: &[Value],
    index: usize,
    name: &'static str,
) -> Result<NodeId, ProcedureError> {
    match &args[index] {
        Value::NodeRef(node_id) => Ok(*node_id),
        other => Err(invalid_argument(format!(
            "{procedure}: expected {name} to be NODE, got {other:?}"
        ))),
    }
}

pub(crate) fn required_node_ref_list(
    procedure: &'static str,
    args: &[Value],
    index: usize,
    name: &'static str,
) -> Result<Vec<NodeId>, ProcedureError> {
    let Value::List(values) = &args[index] else {
        return Err(invalid_argument(format!(
            "{procedure}: expected {name} to be LIST<NODE>, got {:?}",
            args[index]
        )));
    };
    values
        .iter()
        .enumerate()
        .map(|(item_index, value)| match value {
            Value::NodeRef(node_id) => Ok(*node_id),
            other => Err(invalid_argument(format!(
                "{procedure}: expected {name}[{item_index}] to be NODE, got {other:?}"
            ))),
        })
        .collect()
}

pub(crate) fn required_f32_matrix(
    procedure: &'static str,
    args: &[Value],
    index: usize,
    name: &'static str,
    expected_outer_len: usize,
) -> Result<Vec<Vec<f32>>, ProcedureError> {
    let Value::List(rows) = &args[index] else {
        return Err(invalid_argument(format!(
            "{procedure}: expected {name} to be LIST<LIST<FLOAT>>, got {:?}",
            args[index]
        )));
    };
    if rows.len() != expected_outer_len {
        return Err(invalid_argument(format!(
            "{procedure}: {name} length {} does not match node_ids length {expected_outer_len}",
            rows.len()
        )));
    }
    rows.iter()
        .enumerate()
        .map(|(row_index, row)| {
            let Value::List(values) = row else {
                return Err(invalid_argument(format!(
                    "{procedure}: expected {name}[{row_index}] to be LIST<FLOAT>, got {row:?}"
                )));
            };
            values
                .iter()
                .enumerate()
                .map(|(col_index, value)| {
                    numeric_to_f32_at(
                        procedure,
                        &format!("{name}[{row_index}][{col_index}]"),
                        value,
                    )
                })
                .collect()
        })
        .collect()
}

pub(crate) fn required_usize(
    procedure: &'static str,
    args: &[Value],
    index: usize,
    name: &'static str,
) -> Result<usize, ProcedureError> {
    match &args[index] {
        Value::Int(value) if *value >= 0 => usize::try_from(*value)
            .map_err(|_| invalid_argument(format!("{procedure}: {name} is too large"))),
        Value::Int(_) => Err(invalid_argument(format!(
            "{procedure}: {name} must be non-negative"
        ))),
        Value::Uint(value) => usize::try_from(*value)
            .map_err(|_| invalid_argument(format!("{procedure}: {name} is too large"))),
        other => Err(invalid_argument(format!(
            "{procedure}: expected {name} to be INTEGER, got {other:?}"
        ))),
    }
}

pub(crate) fn nullable_option_usize(
    procedure: &'static str,
    args: &[Value],
    index: usize,
    name: &'static str,
) -> Result<Option<usize>, ProcedureError> {
    match &args[index] {
        Value::Null => Ok(None),
        Value::Int(value) if *value >= 0 => usize::try_from(*value)
            .map(Some)
            .map_err(|_| invalid_argument(format!("{procedure}: {name} is too large"))),
        Value::Int(_) => Err(invalid_argument(format!(
            "{procedure}: {name} must be non-negative"
        ))),
        Value::Uint(value) => usize::try_from(*value)
            .map(Some)
            .map_err(|_| invalid_argument(format!("{procedure}: {name} is too large"))),
        other => Err(invalid_argument(format!(
            "{procedure}: expected {name} to be INTEGER or NULL, got {other:?}"
        ))),
    }
}

pub(crate) fn nullable_option_u32(
    procedure: &'static str,
    args: &[Value],
    index: usize,
    name: &'static str,
) -> Result<Option<u32>, ProcedureError> {
    match &args[index] {
        Value::Null => Ok(None),
        Value::Int(value) if *value >= 0 => u32::try_from(*value)
            .map(Some)
            .map_err(|_| invalid_argument(format!("{procedure}: {name} is too large"))),
        Value::Int(_) => Err(invalid_argument(format!(
            "{procedure}: {name} must be non-negative"
        ))),
        Value::Uint(value) => u32::try_from(*value)
            .map(Some)
            .map_err(|_| invalid_argument(format!("{procedure}: {name} is too large"))),
        other => Err(invalid_argument(format!(
            "{procedure}: expected {name} to be INTEGER or NULL, got {other:?}"
        ))),
    }
}

pub(crate) fn nullable_node_ref_list(
    procedure: &'static str,
    args: &[Value],
    index: usize,
    name: &'static str,
) -> Result<Option<Vec<NodeId>>, ProcedureError> {
    match &args[index] {
        Value::Null => Ok(None),
        Value::List(values) => values
            .iter()
            .enumerate()
            .map(|(item_index, value)| match value {
                Value::NodeRef(node_id) => Ok(*node_id),
                other => Err(invalid_argument(format!(
                    "{procedure}: expected {name}[{item_index}] to be NODE, got {other:?}"
                ))),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        other => Err(invalid_argument(format!(
            "{procedure}: expected {name} to be LIST<NODE> or NULL, got {other:?}"
        ))),
    }
}

#[allow(dead_code)]
pub(crate) fn nullable_distance_metric(
    procedure: &'static str,
    args: &[Value],
    index: usize,
    name: &'static str,
) -> Result<Option<DistanceMetric>, ProcedureError> {
    let value = match &args[index] {
        Value::Null => return Ok(None),
        Value::String(value) => value.as_str(),
        Value::ExternalString(value) => value.as_ref(),
        other => {
            return Err(invalid_argument(format!(
                "{procedure}: expected {name} to be STRING or NULL, got {other:?}"
            )));
        }
    };
    let metric = match value.to_ascii_lowercase().as_str() {
        "cosine" => DistanceMetric::Cosine,
        "l2" => DistanceMetric::L2,
        "dot" => DistanceMetric::Dot,
        _ => {
            return Err(invalid_argument(format!(
                "{procedure}: unknown {name} '{value}', expected cosine, l2, or dot"
            )));
        }
    };
    Ok(Some(metric))
}

pub(crate) fn try_filter_from_node_refs(nodes: &[NodeId]) -> Result<RoaringBitmap, ProcedureError> {
    let mut filter = RoaringBitmap::new();
    for node_id in nodes {
        let raw = u32::try_from(node_id.get()).map_err(|_| {
            invalid_argument(format!(
                "filter_nodes contains node id {} above u32::MAX",
                node_id.get()
            ))
        })?;
        filter.insert(raw);
    }
    Ok(filter)
}

fn numeric_to_f32(
    procedure: &'static str,
    name: &'static str,
    item_index: usize,
    value: &Value,
) -> Result<f32, ProcedureError> {
    let converted = match value {
        Value::Float(value) => *value as f32,
        Value::Float32(value) => *value,
        Value::Int(value) => *value as f32,
        Value::Uint(value) => *value as f32,
        other => {
            return Err(invalid_argument(format!(
                "{procedure} expected {name}[{item_index}] to be FLOAT, got {other:?}"
            )));
        }
    };
    Ok(converted)
}

fn numeric_to_f32_at(
    procedure: &'static str,
    location: &str,
    value: &Value,
) -> Result<f32, ProcedureError> {
    let converted = match value {
        Value::Float(value) => *value as f32,
        Value::Float32(value) => *value,
        Value::Int(value) => *value as f32,
        Value::Uint(value) => *value as f32,
        other => {
            return Err(invalid_argument(format!(
                "{procedure}: expected {location} to be FLOAT, got {other:?}"
            )));
        }
    };
    Ok(converted)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROC: &str = "vector.ivf_search";

    #[test]
    fn nullable_option_u32_accepts_null() {
        let args = [Value::Null];

        assert_eq!(
            nullable_option_u32(PROC, &args, 0, "n_probe").expect("null accepted"),
            None
        );
    }

    #[test]
    fn nullable_option_u32_accepts_valid_signed_integer() {
        let args = [Value::Int(32)];

        assert_eq!(
            nullable_option_u32(PROC, &args, 0, "n_probe").expect("int accepted"),
            Some(32)
        );
    }

    #[test]
    fn nullable_option_u32_rejects_negative_integer() {
        let args = [Value::Int(-1)];

        let err = nullable_option_u32(PROC, &args, 0, "n_probe").expect_err("negative rejected");

        assert!(matches!(err, ProcedureError::InvalidArgument { .. }));
    }

    #[test]
    fn nullable_option_u32_rejects_signed_overflow() {
        let args = [Value::Int(i64::from(u32::MAX) + 1)];

        let err = nullable_option_u32(PROC, &args, 0, "n_probe").expect_err("overflow rejected");

        assert!(matches!(err, ProcedureError::InvalidArgument { .. }));
    }

    #[test]
    fn nullable_option_u32_rejects_non_integer() {
        let args = [Value::Bool(true)];

        let err = nullable_option_u32(PROC, &args, 0, "n_probe").expect_err("bool rejected");

        assert!(matches!(err, ProcedureError::InvalidArgument { .. }));
    }

    #[test]
    fn nullable_option_u32_accepts_valid_unsigned_integer() {
        let args = [Value::Uint(17)];

        assert_eq!(
            nullable_option_u32(PROC, &args, 0, "n_probe").expect("uint accepted"),
            Some(17)
        );
    }

    #[test]
    fn nullable_option_u32_rejects_unsigned_overflow() {
        let args = [Value::Uint(u64::from(u32::MAX) + 1)];

        let err = nullable_option_u32(PROC, &args, 0, "n_probe").expect_err("overflow rejected");

        assert!(matches!(err, ProcedureError::InvalidArgument { .. }));
    }

    #[test]
    fn filter_rejects_node_id_above_roaring_range() {
        let err = try_filter_from_node_refs(&[NodeId::new(u64::from(u32::MAX) + 1)])
            .expect_err("overflow rejected");

        assert!(matches!(err, ProcedureError::InvalidArgument { .. }));
    }

    #[test]
    fn nullable_distance_metric_accepts_null() {
        let args = [Value::Null];

        assert_eq!(
            nullable_distance_metric(PROC, &args, 0, "metric").expect("null accepted"),
            None
        );
    }

    #[test]
    fn nullable_distance_metric_accepts_case_insensitive_strings() {
        let cases = [
            ("cosine", DistanceMetric::Cosine),
            ("COSINE", DistanceMetric::Cosine),
            ("l2", DistanceMetric::L2),
            ("L2", DistanceMetric::L2),
            ("dot", DistanceMetric::Dot),
            ("Dot", DistanceMetric::Dot),
        ];

        for (input, expected) in cases {
            let args = [Value::ExternalString(input.into())];
            assert_eq!(
                nullable_distance_metric(PROC, &args, 0, "metric").expect("metric accepted"),
                Some(expected)
            );
        }
    }

    #[test]
    fn nullable_distance_metric_rejects_unknown_metric() {
        let args = [Value::ExternalString("manhattan".into())];

        let err = nullable_distance_metric(PROC, &args, 0, "metric")
            .expect_err("unknown metric rejected");

        assert!(matches!(err, ProcedureError::InvalidArgument { .. }));
    }
}
