use selene_core::{IStr, LabelSet, PropertyValueType};
use selene_graph::{EdgeTypeDef, GraphTypeDef, NodeTypeDef, PropertyTypeDef};

use crate::{
    GqlType, MutationStatement, RecordType, SetItem, SourceSpan, ValueExpr,
    analyze::{
        ast::{AnalyzedStatement, AnalyzedStatementKind},
        binding::BindingId,
        error::AnalysisError,
        types::AnalyzedType,
        write_set::WriteKind,
    },
};

pub(super) fn validate_property_values(
    graph_type: &GraphTypeDef,
    declared_in: IStr,
    declarations: &[PropertyTypeDef],
    properties: &[(IStr, ValueExpr)],
    analyzed: &AnalyzedStatement,
) -> Result<(), AnalysisError> {
    for (key, value) in properties {
        let declaration = declarations.iter().find(|decl| decl.name == *key).ok_or(
            AnalysisError::SchemaUndeclaredProperty {
                property: *key,
                declared_in,
                graph_type: graph_type.name,
                span: value.span(),
            },
        )?;
        validate_one_property_value(declared_in, declaration, *key, value, analyzed)?;
    }
    Ok(())
}

pub(super) fn validate_one_property_value(
    declared_in: IStr,
    declaration: &PropertyTypeDef,
    key: IStr,
    value: &ValueExpr,
    analyzed: &AnalyzedStatement,
) -> Result<(), AnalysisError> {
    let Some(id) = analyzed.expr_ids.get(value) else {
        return Ok(());
    };
    match analyzed.expr_types.get(id) {
        AnalyzedType::Dynamic => Ok(()),
        AnalyzedType::Resolved(GqlType::Null) if !declaration.required => Ok(()),
        AnalyzedType::Resolved(GqlType::Null) => {
            Err(AnalysisError::SchemaRequiredPropertyMissing {
                property: key,
                declared_in,
                span: value.span(),
            })
        }
        AnalyzedType::Resolved(found)
            if property_type_compatible(declaration.value_type, found) =>
        {
            Ok(())
        }
        AnalyzedType::Resolved(found) => Err(AnalysisError::SchemaPropertyTypeMismatch {
            property: key,
            declared_in,
            expected: declaration.value_type,
            found: found.clone(),
            span: value.span(),
        }),
    }
}

pub(super) struct RequiredPropertyCheck<'a> {
    pub(super) declared_in: IStr,
    pub(super) declarations: &'a [PropertyTypeDef],
    pub(super) properties: &'a [(IStr, ValueExpr)],
    pub(super) stmt_index: usize,
    pub(super) binding: Option<BindingId>,
    pub(super) span: SourceSpan,
    pub(super) analyzed: &'a AnalyzedStatement,
}

pub(super) fn validate_required_properties(
    check: RequiredPropertyCheck<'_>,
) -> Result<(), AnalysisError> {
    for declaration in check.declarations.iter().filter(|decl| decl.required) {
        if check
            .properties
            .iter()
            .any(|(key, _)| *key == declaration.name)
        {
            continue;
        }
        if check.binding.is_some_and(|binding| {
            required_property_supplied(check.stmt_index, binding, declaration.name, check.analyzed)
        }) {
            continue;
        }
        return Err(AnalysisError::SchemaRequiredPropertyMissing {
            property: declaration.name,
            declared_in: check.declared_in,
            span: check.span,
        });
    }
    Ok(())
}

fn required_property_supplied(
    stmt_index: usize,
    binding: BindingId,
    property: IStr,
    analyzed: &AnalyzedStatement,
) -> bool {
    analyzed.write_set.as_ref().is_some_and(|write_set| {
        write_set.entries.iter().any(|entry| {
            entry.statement_index > stmt_index
                && matches!(
                    &entry.kind,
                    WriteKind::SetProperty { target, key, .. }
                        if *target == binding && *key == property
                )
        })
    })
}

pub(super) fn find_set_value(
    analyzed: &AnalyzedStatement,
    value_span: SourceSpan,
) -> Option<&ValueExpr> {
    let AnalyzedStatementKind::Mutate(pipeline) = &analyzed.statement else {
        return None;
    };
    for statement in &pipeline.statements {
        let MutationStatement::Set(items) = statement else {
            continue;
        };
        for item in items {
            match item {
                SetItem::Property { value, .. } if value.span() == value_span => {
                    return Some(value);
                }
                SetItem::PropertyMerge { properties, .. } => {
                    if let Some((_, value)) = properties
                        .iter()
                        .find(|(_, value)| value.span() == value_span)
                    {
                        return Some(value);
                    }
                }
                _ => {}
            }
        }
    }
    None
}

pub(super) enum PropertyAgreement<'a> {
    Declared(&'a PropertyTypeDef, IStr),
    Undeclared(IStr),
    Disagree,
}

pub(super) fn property_agreement<'a>(
    types: &[&'a NodeTypeDef],
    key: IStr,
) -> PropertyAgreement<'a> {
    let Some(first_type) = types.first() else {
        return PropertyAgreement::Disagree;
    };
    let mut agreed: Option<(&PropertyTypeDef, IStr)> = None;
    let mut saw_undeclared = false;
    for node_type in types {
        match node_type.properties.iter().find(|decl| decl.name == key) {
            Some(_) if saw_undeclared => return PropertyAgreement::Disagree,
            Some(decl) => {
                if let Some((prior, _)) = agreed
                    && (prior.value_type != decl.value_type || prior.required != decl.required)
                {
                    return PropertyAgreement::Disagree;
                }
                agreed = Some((decl, node_type.name));
            }
            None if agreed.is_some() => return PropertyAgreement::Disagree,
            None => saw_undeclared = true,
        }
    }
    agreed
        .map(|(decl, name)| PropertyAgreement::Declared(decl, name))
        .unwrap_or(PropertyAgreement::Undeclared(first_type.name))
}

pub(super) fn validate_declared_property(
    edge_type: &EdgeTypeDef,
    key: IStr,
    graph_type: IStr,
    span: SourceSpan,
) -> Result<&PropertyTypeDef, AnalysisError> {
    edge_type
        .properties
        .iter()
        .find(|decl| decl.name == key)
        .ok_or(AnalysisError::SchemaUndeclaredProperty {
            property: key,
            declared_in: edge_type.name,
            graph_type,
            span,
        })
}

pub(super) fn undeclared_property(
    property: IStr,
    declared_in: IStr,
    graph_type: IStr,
    span: SourceSpan,
) -> Result<(), AnalysisError> {
    Err(AnalysisError::SchemaUndeclaredProperty {
        property,
        declared_in,
        graph_type,
        span,
    })
}

pub(super) fn validate_node_label_transition(
    types: Vec<&NodeTypeDef>,
    graph_type: &GraphTypeDef,
    span: SourceSpan,
    mutate: impl Fn(&mut LabelSet),
) -> Result<(), AnalysisError> {
    let mut first_invalid = None;
    for node_type in &types {
        let mut labels = node_type.key_labels.clone();
        mutate(&mut labels);
        if graph_type.find_node_type(&labels).is_none() {
            first_invalid.get_or_insert(labels);
        }
    }
    if let Some(labels) = first_invalid {
        return Err(AnalysisError::SchemaUnknownNodeType {
            labels,
            graph_type: graph_type.name,
            span,
        });
    }
    Ok(())
}

pub(super) fn property_type_compatible(declared: PropertyValueType, found: &GqlType) -> bool {
    use GqlType as G;
    use PropertyValueType as P;
    matches!(
        (declared, found),
        (P::Bool, G::Boolean)
            | (
                P::Int,
                G::Integer | G::Int8 | G::Int16 | G::Int32 | G::Int64 | G::SmallInt | G::BigInt
            )
            | (P::Uint, G::Uint8 | G::Uint16 | G::Uint32 | G::Uint64)
            | (P::Int128, G::Int128)
            | (P::Uint128, G::Uint128)
            | (P::Float, G::Float | G::Float64)
            | (P::Float32, G::Float32)
            | (P::Decimal, G::Decimal)
            | (P::String, G::String)
            | (P::Uuid, G::Uuid)
            | (P::Bytes, G::Bytes)
            | (P::List, G::List(_))
            // Every record property declaration — open `RECORD` and closed `RECORD{..}`
            // alike — lowers to `P::RecordTyped` (catalog/property.rs), while a `RECORD{..}`
            // constructor always analyzes to the OPEN record type (bind/expr.rs). Accept the
            // open literal at this coarse type-compat gate; closed-record field-name-set +
            // per-field conformance is enforced at commit time by `RecordFieldTypes::matches`
            // → G2000 (ISO 39075:2024 §4.15.4). Without the open arm, a record write into any
            // RECORD-typed property in a GG02 graph is rejected at analysis even though the
            // runtime accepts and structurally validates it.
            | (P::Record, G::Record(RecordType::Open))
            | (P::RecordTyped, G::Record(_))
            | (P::ZonedDateTime, G::ZonedDateTime)
            | (P::LocalDateTime, G::LocalDateTime)
            | (P::Date, G::Date)
            | (P::ZonedTime, G::ZonedTime)
            | (P::LocalTime, G::LocalTime)
            | (P::Duration, G::Duration)
            | (P::Null, G::Null)
            | (P::Path, G::Path)
            | (P::NodeRef, G::NodeRef)
            | (P::EdgeRef, G::EdgeRef)
            | (P::GraphRef, G::GraphRef)
            | (P::TableRef, G::TableRef)
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use selene_core::{BindingTableId, EdgeId, GraphId, NodeId, Value, intern_with_admission};

    use super::*;

    #[test]
    fn property_type_compatibility_is_storage_tag_exact() {
        assert!(property_type_compatible(
            PropertyValueType::Int,
            &GqlType::Integer
        ));
        assert!(property_type_compatible(
            PropertyValueType::Int,
            &GqlType::BigInt
        ));
        assert!(!property_type_compatible(
            PropertyValueType::Int,
            &GqlType::Uint64
        ));
        assert!(!property_type_compatible(
            PropertyValueType::Int,
            &GqlType::Int128
        ));

        assert!(property_type_compatible(
            PropertyValueType::Uint,
            &GqlType::Uint32
        ));
        assert!(!property_type_compatible(
            PropertyValueType::Uint,
            &GqlType::Integer
        ));
        assert!(!property_type_compatible(
            PropertyValueType::Uint,
            &GqlType::Uint128
        ));

        assert!(property_type_compatible(
            PropertyValueType::Float,
            &GqlType::Float64
        ));
        assert!(!property_type_compatible(
            PropertyValueType::Float,
            &GqlType::Float32
        ));
        assert!(property_type_compatible(
            PropertyValueType::Float32,
            &GqlType::Float32
        ));
        assert!(!property_type_compatible(
            PropertyValueType::Float32,
            &GqlType::Float
        ));

        assert!(property_type_compatible(
            PropertyValueType::Bytes,
            &GqlType::Bytes
        ));
        assert!(!property_type_compatible(
            PropertyValueType::Bytes,
            &GqlType::String
        ));

        assert!(property_type_compatible(
            PropertyValueType::List,
            &GqlType::List(Box::new(GqlType::Integer))
        ));
        assert!(property_type_compatible(
            PropertyValueType::Record,
            &GqlType::Record(RecordType::Open)
        ));
        assert!(property_type_compatible(
            PropertyValueType::RecordTyped,
            &GqlType::Record(RecordType::Closed(Vec::new()))
        ));
        // A `RECORD{..}` constructor always analyzes to the OPEN record type, so a
        // `RecordTyped`-declared property (the tag every record declaration lowers to)
        // must accept the open literal at this coarse gate; closed-field conformance is
        // deferred to the runtime (`RecordFieldTypes::matches` → G2000). Regression pin
        // for the analyzer/runtime divergence that made typed-RECORD writes unexecutable.
        assert!(property_type_compatible(
            PropertyValueType::RecordTyped,
            &GqlType::Record(RecordType::Open)
        ));
        // The gate stays closed for a genuine mismatch: a record literal does not satisfy
        // a scalar declaration.
        assert!(!property_type_compatible(
            PropertyValueType::Int,
            &GqlType::Record(RecordType::Open)
        ));
        assert!(property_type_compatible(
            PropertyValueType::Null,
            &GqlType::Null
        ));
        assert!(property_type_compatible(
            PropertyValueType::Uuid,
            &GqlType::Uuid
        ));
        assert!(!property_type_compatible(
            PropertyValueType::Uuid,
            &GqlType::String
        ));
    }

    #[test]
    fn property_type_compatibility_matches_runtime_storage_samples() {
        let zoned = "2026-05-07T12:34:56-04:00[America/New_York]";
        let samples = [
            (PropertyValueType::Bool, GqlType::Boolean, Value::Bool(true)),
            (PropertyValueType::Int, GqlType::Integer, Value::Int(-1)),
            (PropertyValueType::Uint, GqlType::Uint64, Value::Uint(1)),
            (
                PropertyValueType::Int128,
                GqlType::Int128,
                Value::Int128(-1),
            ),
            (
                PropertyValueType::Uint128,
                GqlType::Uint128,
                Value::Uint128(1),
            ),
            (
                PropertyValueType::Float,
                GqlType::Float64,
                Value::Float(1.25),
            ),
            (
                PropertyValueType::Float32,
                GqlType::Float32,
                Value::Float32(1.5),
            ),
            (
                PropertyValueType::Decimal,
                GqlType::Decimal,
                Value::Decimal("123.45".parse().unwrap()),
            ),
            (
                PropertyValueType::String,
                GqlType::String,
                Value::String(intern_with_admission("schema.type.string").unwrap().0),
            ),
            (
                PropertyValueType::Uuid,
                GqlType::Uuid,
                Value::Uuid(uuid::Uuid::nil()),
            ),
            (
                PropertyValueType::Bytes,
                GqlType::Bytes,
                Value::Bytes(Arc::from([1_u8, 2, 3])),
            ),
            (
                PropertyValueType::NodeRef,
                GqlType::NodeRef,
                Value::NodeRef(NodeId::new(1)),
            ),
            (
                PropertyValueType::EdgeRef,
                GqlType::EdgeRef,
                Value::EdgeRef(EdgeId::new(1)),
            ),
            (
                PropertyValueType::GraphRef,
                GqlType::GraphRef,
                Value::GraphRef(GraphId::new(1)),
            ),
            (
                PropertyValueType::TableRef,
                GqlType::TableRef,
                Value::TableRef(BindingTableId::new(1)),
            ),
            (
                PropertyValueType::ZonedDateTime,
                GqlType::ZonedDateTime,
                Value::ZonedDateTime(zoned.parse().unwrap()),
            ),
            (
                PropertyValueType::LocalDateTime,
                GqlType::LocalDateTime,
                Value::LocalDateTime("2026-05-07T12:34:56".parse().unwrap()),
            ),
            (
                PropertyValueType::Date,
                GqlType::Date,
                Value::Date("2026-05-07".parse().unwrap()),
            ),
            (
                PropertyValueType::ZonedTime,
                GqlType::ZonedTime,
                Value::ZonedTime(zoned.parse().unwrap()),
            ),
            (
                PropertyValueType::LocalTime,
                GqlType::LocalTime,
                Value::LocalTime("12:34:56".parse().unwrap()),
            ),
            (
                PropertyValueType::Duration,
                GqlType::Duration,
                Value::Duration("PT1H2S".parse().unwrap()),
            ),
            (PropertyValueType::Null, GqlType::Null, Value::Null),
        ];

        for (declared, gql_type, value) in samples {
            assert!(declared.matches(&value));
            assert!(property_type_compatible(declared, &gql_type));
        }
    }
}
