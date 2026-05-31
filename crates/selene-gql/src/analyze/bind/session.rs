//! Session-control bind handling (ISO/IEC 39075:2024 section 7).
//!
//! Session commands carry no binding-variable scope: `SESSION SET VALUE`
//! evaluates its right-hand side against an empty binding row (restricted to a
//! `<value specification>` per ISO section 7.1 Conformance Rule, so GS14 is not
//! claimed), and the RESET / CLOSE forms mutate session state only. The bind
//! pass is therefore a no-op, mirroring transaction-control binding.

use crate::SourceSpan;

use super::BindContext;

pub(crate) fn bind_session_command(_ctx: &mut BindContext, _span: SourceSpan) {}
