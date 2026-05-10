use selene_core::Value;

use crate::{
    Aggregate, BindingTableColumn, ProjectExpr,
    runtime::{Binding, BindingTable, ExecutorError, TxContext, evaluator, value_compare},
};

use super::aggregate::{self, AggregateSlot};

pub(super) fn execute(
    keys: &[ProjectExpr],
    aggregates: &[Aggregate],
    table: BindingTable,
    ctx: &TxContext<'_>,
) -> Result<BindingTable, ExecutorError> {
    let input_schema = table.schema().clone();
    let output_schema = output_schema(&input_schema, aggregates);
    let mut groups = Vec::<Group>::new();

    for row in table.rows() {
        let key = evaluate_key_tuple(keys, row, &input_schema, ctx)?;
        let index = groups
            .iter()
            .position(|group| key_tuples_equal(&group.key, &key))
            .map(Ok)
            .unwrap_or_else(|| {
                groups.push(Group::new(key.clone(), row.clone(), aggregates)?);
                Ok(groups.len() - 1)
            })?;
        groups[index].observe(row, &input_schema, ctx)?;
    }

    if keys.is_empty() && groups.is_empty() {
        let representative = Binding::new(
            input_schema
                .columns
                .iter()
                .map(|_| Value::Null)
                .collect::<Vec<_>>(),
        );
        groups.push(Group::new(Vec::new(), representative, aggregates)?);
    }

    let rows = groups
        .into_iter()
        .map(Group::finalize)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(BindingTable::new(output_schema, rows))
}

struct Group {
    key: Vec<Value>,
    representative: Binding,
    aggregates: Vec<AggregateSlot>,
}

impl Group {
    fn new(
        key: Vec<Value>,
        representative: Binding,
        aggregates: &[Aggregate],
    ) -> Result<Self, ExecutorError> {
        let aggregates = aggregates
            .iter()
            .map(AggregateSlot::new)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            key,
            representative,
            aggregates,
        })
    }

    fn observe(
        &mut self,
        row: &Binding,
        schema: &crate::BindingTableSchema,
        ctx: &TxContext<'_>,
    ) -> Result<(), ExecutorError> {
        for aggregate in &mut self.aggregates {
            aggregate.observe(row, schema, ctx)?;
        }
        Ok(())
    }

    fn finalize(self) -> Result<Binding, ExecutorError> {
        let mut values = self.representative.values().to_vec();
        for aggregate in self.aggregates {
            values.extend(aggregate.finalize_values()?);
        }
        Ok(Binding::new(values))
    }
}

fn output_schema(
    input_schema: &crate::BindingTableSchema,
    aggregates: &[Aggregate],
) -> crate::BindingTableSchema {
    let mut schema = input_schema.clone();
    schema
        .columns
        .extend(aggregates.iter().flat_map(|aggregate| {
            aggregate::output_names(aggregate)
                .into_iter()
                .map(|name| BindingTableColumn {
                    name: Some(name),
                    hidden: None,
                    ty: aggregate.ty.clone(),
                })
                .collect::<Vec<_>>()
        }));
    schema
}

fn evaluate_key_tuple(
    keys: &[ProjectExpr],
    row: &Binding,
    schema: &crate::BindingTableSchema,
    ctx: &TxContext<'_>,
) -> Result<Vec<Value>, ExecutorError> {
    keys.iter()
        .map(|key| evaluator::evaluate(&key.expr, row, schema, ctx))
        .collect()
}

fn key_tuples_equal(lhs: &[Value], rhs: &[Value]) -> bool {
    lhs.len() == rhs.len()
        && lhs
            .iter()
            .zip(rhs)
            .all(|(lhs, rhs)| key_values_equal(lhs, rhs))
}

fn key_values_equal(lhs: &Value, rhs: &Value) -> bool {
    match (lhs, rhs) {
        (Value::Null, Value::Null) => true,
        (Value::Null, _) | (_, Value::Null) => false,
        _ => value_compare::equal_non_null(lhs, rhs),
    }
}
