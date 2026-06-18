use selene_core::{DbString, Value};
use smallvec::SmallVec;

use crate::{
    AnalyzedType, BindingTableColumn, GqlType, ProjectExpr, RowExpansionPosition,
    RowExpansionPositionKind, SourceSpan,
    runtime::{Binding, BindingTable, DataExceptionSubclass, EvalCtx, ExecutorError, evaluator},
};

pub(super) fn execute(
    source: &ProjectExpr,
    alias: DbString,
    position: Option<RowExpansionPosition>,
    span: SourceSpan,
    table: BindingTable,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<BindingTable, ExecutorError> {
    let (input_schema, input_rows) = table.into_parts();
    let mut output_schema = input_schema.clone();
    output_schema.columns.push(BindingTableColumn {
        name: Some(alias),
        hidden: None,
        ty: element_type(&source.ty),
    });
    if let Some(position) = &position {
        output_schema.columns.push(BindingTableColumn {
            name: Some(position.alias.clone()),
            hidden: None,
            ty: AnalyzedType::Resolved(GqlType::Integer),
        });
    }

    let mut rows = Vec::new();
    let mut rows_since_check = 0;
    for row in input_rows {
        ctx.tx.check_cancellation_stride(&mut rows_since_check, 1)?;
        match evaluator::evaluate(&source.expr, &row, &input_schema, ctx)? {
            Value::List(values) => {
                rows.reserve(values.len());
                for (index, value) in values.into_iter().enumerate() {
                    ctx.tx.check_cancellation_stride(&mut rows_since_check, 1)?;
                    let mut output = row.cloned_values();
                    output.push(value);
                    if let Some(position) = &position {
                        output.push(position_value(position.kind, index, span)?);
                    }
                    rows.push(Binding::from_parts(output, SmallVec::new()));
                }
            }
            Value::Null => {}
            _ => {
                return Err(ExecutorError::data_exception(
                    DataExceptionSubclass::InvalidValueType,
                    "row expansion requires a list value",
                    span,
                ));
            }
        }
    }
    Ok(BindingTable::new(output_schema, rows))
}

fn position_value(
    kind: RowExpansionPositionKind,
    index: usize,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    let offset = i64::try_from(index).map_err(|_| {
        ExecutorError::data_exception(
            DataExceptionSubclass::NumericValueOutOfRange,
            "row expansion position exceeds INTEGER range",
            span,
        )
    })?;
    let value = match kind {
        RowExpansionPositionKind::Offset => offset,
        RowExpansionPositionKind::Ordinality => offset.checked_add(1).ok_or_else(|| {
            ExecutorError::data_exception(
                DataExceptionSubclass::NumericValueOutOfRange,
                "row expansion position exceeds INTEGER range",
                span,
            )
        })?,
    };
    Ok(Value::Int(value))
}

fn element_type(ty: &AnalyzedType) -> AnalyzedType {
    match ty {
        AnalyzedType::Resolved(GqlType::List(inner))
        | AnalyzedType::Resolved(GqlType::BoundedList {
            element_type: inner,
            ..
        }) => AnalyzedType::Resolved((**inner).clone()),
        _ => AnalyzedType::DYNAMIC,
    }
}
