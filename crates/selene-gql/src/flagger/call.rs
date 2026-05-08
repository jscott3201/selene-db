//! Procedure-call Flagger walk.

use selene_core::feature_register::FeatureId;

use crate::{ProcedureCall, error::ParserError};

use super::{check_feature, expr};

pub(crate) fn procedure_call(call: &ProcedureCall) -> Result<(), ParserError> {
    check_feature(FeatureId::GP04, call.span)?;
    for arg in &call.args {
        expr::value(arg)?;
    }
    Ok(())
}
