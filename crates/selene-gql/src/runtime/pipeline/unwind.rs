use selene_core::{IStr, Value};

use crate::{
    AnalyzedType, BindingTableColumn, GqlType, ProjectExpr, SourceSpan,
    runtime::{Binding, BindingTable, ExecutorError, TxContext, evaluator},
};

pub(super) fn execute(
    source: &ProjectExpr,
    alias: IStr,
    span: SourceSpan,
    table: BindingTable,
    ctx: &TxContext<'_>,
) -> Result<BindingTable, ExecutorError> {
    let input_schema = table.schema().clone();
    let mut output_schema = input_schema.clone();
    output_schema.columns.push(BindingTableColumn {
        name: Some(alias),
        hidden: None,
        ty: element_type(&source.ty),
    });

    let mut rows = Vec::new();
    for row in table.rows() {
        match evaluator::evaluate(&source.expr, row, &input_schema, ctx)? {
            Value::List(values) => {
                for value in values {
                    let mut output = row.values().to_vec();
                    output.push(value);
                    rows.push(Binding::new(output));
                }
            }
            Value::Null => {}
            _ => {
                return Err(ExecutorError::DataException {
                    message: "UNWIND requires a list value".to_owned(),
                    span,
                });
            }
        }
    }
    Ok(BindingTable::new(output_schema, rows))
}

fn element_type(ty: &AnalyzedType) -> AnalyzedType {
    match ty {
        AnalyzedType::Resolved(GqlType::List(inner)) => AnalyzedType::Resolved((**inner).clone()),
        _ => AnalyzedType::DYNAMIC,
    }
}
