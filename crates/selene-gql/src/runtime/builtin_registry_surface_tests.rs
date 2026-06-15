use selene_core::{DbString, db_string};

use super::*;
use crate::{ProcedureMutability, ProcedureRegistry, ProcedureTier};

fn name(segments: &[&str]) -> Vec<DbString> {
    segments
        .iter()
        .map(|segment| db_string(segment).expect("string fits DB string cap"))
        .collect()
}

#[path = "builtin_registry_surface_tests/general.rs"]
mod general;
#[path = "builtin_registry_surface_tests/vector.rs"]
mod vector;
