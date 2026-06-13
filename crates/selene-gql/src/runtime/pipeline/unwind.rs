use selene_core::{DbString, Value};

use crate::{
    AnalyzedType, BindingTableColumn, GqlType, ProjectExpr, SourceSpan,
    runtime::{Binding, BindingTable, DataExceptionSubclass, EvalCtx, ExecutorError, evaluator},
};

pub(super) fn execute(
    source: &ProjectExpr,
    alias: DbString,
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

    let mut rows = Vec::new();
    let mut rows_since_check = 0;
    for row in input_rows {
        ctx.tx.check_cancellation_stride(&mut rows_since_check, 1)?;
        match evaluator::evaluate(&source.expr, &row, &input_schema, ctx)? {
            Value::List(values) => {
                for value in values {
                    ctx.tx.check_cancellation_stride(&mut rows_since_check, 1)?;
                    let mut output = row.values().to_vec();
                    output.push(value);
                    rows.push(Binding::new(output));
                }
            }
            Value::Null => {}
            _ => {
                return Err(ExecutorError::data_exception(
                    DataExceptionSubclass::InvalidValueType,
                    "UNWIND requires a list value",
                    span,
                ));
            }
        }
    }
    Ok(BindingTable::new(output_schema, rows))
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
