//! Procedure-call Flagger walk.

use selene_core::feature_register::FeatureId;

use crate::ProcedureCall;

use super::{FeatureUse, expr, record_feature};

pub(crate) fn procedure_call(call: &ProcedureCall, uses: &mut Vec<FeatureUse>) {
    record_feature(uses, FeatureId::GP04, call.span);
    for arg in &call.args {
        expr::value(arg, uses);
    }
}
