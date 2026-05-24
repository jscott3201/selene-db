//! Identity and length scalar function evaluation.

use std::sync::Arc;

use selene_core::Value;

use crate::{
    SourceSpan,
    runtime::{DataExceptionSubclass, ExecutorError},
};

use super::binary_ops::data_exception_with;

pub(super) fn eval_element_id(args: Vec<Value>, span: SourceSpan) -> Result<Value, ExecutorError> {
    match args.into_iter().next().expect("arity checked") {
        Value::Null => Ok(Value::Null),
        Value::NodeRef(id) => Ok(Value::ExternalString(Arc::from(id.to_string()))),
        Value::EdgeRef(id) => Ok(Value::ExternalString(Arc::from(id.to_string()))),
        Value::List(_) => data_exception_with(
            DataExceptionSubclass::InvalidValueType,
            "element_id argument is not a singleton element reference",
            span,
        ),
        _ => data_exception_with(
            DataExceptionSubclass::InvalidValueType,
            "element_id argument is not an element reference",
            span,
        ),
    }
}
