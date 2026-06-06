//! VECTOR target support for explicit `CAST`.

use rust_decimal::prelude::ToPrimitive;
use selene_core::{CoreError, Value, VectorValue};

use crate::{
    SourceSpan,
    runtime::{DataExceptionSubclass, ExecutorError},
};

use super::non_iso_combination;

pub(super) fn cast_to_vector(value: Value, span: SourceSpan) -> Result<Value, ExecutorError> {
    match value {
        Value::Vector(vector) => Ok(Value::Vector(vector)),
        Value::List(items) => {
            let mut components = Vec::with_capacity(items.len());
            for (index, item) in items.into_iter().enumerate() {
                components.push(component_to_f32(item, index, span)?);
            }
            VectorValue::new(components)
                .map(Value::Vector)
                .map_err(|err| vector_error(err, span))
        }
        _ => Err(non_iso_combination(
            "CAST to VECTOR requires a VECTOR or LIST<numeric> source",
            span,
        )),
    }
}

fn component_to_f32(value: Value, index: usize, span: SourceSpan) -> Result<f32, ExecutorError> {
    let component = match value {
        #[allow(clippy::cast_precision_loss)]
        Value::Int(value) => value as f32,
        #[allow(clippy::cast_precision_loss)]
        Value::Uint(value) => value as f32,
        #[allow(clippy::cast_precision_loss)]
        Value::Int128(value) => value as f32,
        #[allow(clippy::cast_precision_loss)]
        Value::Uint128(value) => value as f32,
        #[allow(clippy::cast_possible_truncation)]
        Value::Float(value) => value as f32,
        Value::Float32(value) => value,
        Value::Decimal(value) => value
            .to_f32()
            .ok_or_else(|| component_out_of_range(index, span))?,
        _ => return Err(invalid_component_type(index, span)),
    };
    if component.is_finite() {
        Ok(component)
    } else {
        Err(component_out_of_range(index, span))
    }
}

fn vector_error(error: CoreError, span: SourceSpan) -> ExecutorError {
    match error {
        CoreError::VectorComponentNotFinite { index, .. } => component_out_of_range(index, span),
        CoreError::VectorEmpty | CoreError::VectorTooLarge { .. } => {
            non_iso_combination("CAST to VECTOR produced an invalid VECTOR dimension", span)
        }
        _ => ExecutorError::data_exception(
            DataExceptionSubclass::DataException,
            format!("unexpected VECTOR construction error during CAST: {error}"),
            span,
        ),
    }
}

fn invalid_component_type(index: usize, span: SourceSpan) -> ExecutorError {
    ExecutorError::data_exception(
        DataExceptionSubclass::InvalidValueType,
        format!("VECTOR component {index} is not numeric"),
        span,
    )
}

fn component_out_of_range(index: usize, span: SourceSpan) -> ExecutorError {
    ExecutorError::data_exception(
        DataExceptionSubclass::NumericValueOutOfRange,
        format!("VECTOR component {index} has no finite FLOAT32 image"),
        span,
    )
}
