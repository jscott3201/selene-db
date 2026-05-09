#![allow(missing_docs)]
#![allow(dead_code)]

use std::time::Duration;

use criterion::Criterion;
use selene_testing::{BenchFixture, BenchProfile};

pub(crate) fn criterion_config() -> Criterion {
    let profile = BenchProfile::from_env();
    Criterion::default()
        .sample_size(profile.sample_size())
        .warm_up_time(Duration::from_millis(100))
        .measurement_time(Duration::from_millis(match profile {
            BenchProfile::Quick => 500,
            BenchProfile::Full | BenchProfile::Stress => 1_500,
            _ => 500,
        }))
}

pub(crate) fn fixtures() -> Vec<BenchFixture> {
    BenchProfile::from_env()
        .scales()
        .iter()
        .copied()
        .map(BenchFixture::build)
        .collect()
}
