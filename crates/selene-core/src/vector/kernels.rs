use wide::f64x4;

use crate::{CoreError, CoreResult};

const F64X4_COMPONENTS: usize = 4;
const F64X4_UNROLL: usize = 4;
const F64X4_UNROLLED_COMPONENTS: usize = F64X4_COMPONENTS * F64X4_UNROLL;
const F64X4_COSINE_UNROLL: usize = 2;
const F64X4_COSINE_UNROLLED_COMPONENTS: usize = F64X4_COMPONENTS * F64X4_COSINE_UNROLL;
const SQUARED_EUCLIDEAN_UNROLL_MIN_COMPONENTS: usize = 512;
const COSINE_COMPONENTS_UNROLL_MIN_COMPONENTS: usize = 1024;

pub(super) fn squared_euclidean(lhs: &[f32], rhs: &[f32]) -> f64 {
    if lhs.len() < SQUARED_EUCLIDEAN_UNROLL_MIN_COMPONENTS {
        return squared_euclidean_single_chain(lhs, rhs);
    }

    let mut chunks_lhs = lhs.chunks_exact(F64X4_UNROLLED_COMPONENTS);
    let mut chunks_rhs = rhs.chunks_exact(F64X4_UNROLLED_COMPONENTS);
    let mut distance0 = f64x4::ZERO;
    let mut distance1 = f64x4::ZERO;
    let mut distance2 = f64x4::ZERO;
    let mut distance3 = f64x4::ZERO;
    for (lhs, rhs) in chunks_lhs.by_ref().zip(chunks_rhs.by_ref()) {
        add_squared_euclidean(&mut distance0, &lhs[0..4], &rhs[0..4]);
        add_squared_euclidean(&mut distance1, &lhs[4..8], &rhs[4..8]);
        add_squared_euclidean(&mut distance2, &lhs[8..12], &rhs[8..12]);
        add_squared_euclidean(&mut distance3, &lhs[12..16], &rhs[12..16]);
    }

    let mut chunks_lhs = chunks_lhs.remainder().chunks_exact(F64X4_COMPONENTS);
    let mut chunks_rhs = chunks_rhs.remainder().chunks_exact(F64X4_COMPONENTS);
    let mut distance_tail = f64x4::ZERO;
    for (lhs, rhs) in chunks_lhs.by_ref().zip(chunks_rhs.by_ref()) {
        add_squared_euclidean(&mut distance_tail, lhs, rhs);
    }

    let mut distance =
        reduce_four(distance0, distance1, distance2, distance3) + distance_tail.reduce_add();
    for (&lhs, &rhs) in chunks_lhs.remainder().iter().zip(chunks_rhs.remainder()) {
        let delta = f64::from(lhs) - f64::from(rhs);
        distance += delta * delta;
    }
    distance
}

fn squared_euclidean_single_chain(lhs: &[f32], rhs: &[f32]) -> f64 {
    let mut chunks_lhs = lhs.chunks_exact(F64X4_COMPONENTS);
    let mut chunks_rhs = rhs.chunks_exact(F64X4_COMPONENTS);
    let mut distance = f64x4::ZERO;
    for (lhs, rhs) in chunks_lhs.by_ref().zip(chunks_rhs.by_ref()) {
        add_squared_euclidean(&mut distance, lhs, rhs);
    }
    let mut distance = distance.reduce_add();
    for (&lhs, &rhs) in chunks_lhs.remainder().iter().zip(chunks_rhs.remainder()) {
        let delta = f64::from(lhs) - f64::from(rhs);
        distance += delta * delta;
    }
    distance
}

pub(super) fn cosine_distance(lhs: &[f32], rhs: &[f32]) -> CoreResult<f64> {
    let (lhs_norm, rhs_norm, dot) = cosine_components(lhs, rhs);
    if lhs_norm == 0.0 {
        return Err(CoreError::VectorZeroNorm { side: "lhs" });
    }
    cosine_distance_with_components(lhs_norm, rhs_norm, dot)
}

pub(super) fn cosine_distance_with_lhs_norm(
    lhs: &[f32],
    rhs: &[f32],
    lhs_norm: f64,
) -> CoreResult<f64> {
    let (rhs_norm, dot) = norm_and_dot(lhs, rhs);
    cosine_distance_with_components(lhs_norm, rhs_norm, dot)
}

pub(super) fn cosine_distance_with_norms(
    lhs: &[f32],
    rhs: &[f32],
    lhs_norm: f64,
    rhs_norm: f64,
) -> CoreResult<f64> {
    let rhs_norm = validate_precomputed_squared_norm(rhs_norm, "rhs")?;
    cosine_distance_with_components(lhs_norm, rhs_norm, dot(lhs, rhs))
}

fn cosine_distance_with_components(lhs_norm: f64, rhs_norm: f64, dot: f64) -> CoreResult<f64> {
    if rhs_norm == 0.0 {
        return Err(CoreError::VectorZeroNorm { side: "rhs" });
    }
    let similarity = dot / (lhs_norm.sqrt() * rhs_norm.sqrt());
    Ok(1.0 - similarity.clamp(-1.0, 1.0))
}

pub(super) fn validate_precomputed_squared_norm(norm: f64, side: &'static str) -> CoreResult<f64> {
    if norm > 0.0 && norm.is_finite() {
        Ok(norm)
    } else {
        Err(CoreError::VectorZeroNorm { side })
    }
}

fn cosine_components(lhs: &[f32], rhs: &[f32]) -> (f64, f64, f64) {
    if lhs.len() < COSINE_COMPONENTS_UNROLL_MIN_COMPONENTS {
        return cosine_components_single_chain(lhs, rhs);
    }

    let mut chunks_lhs = lhs.chunks_exact(F64X4_COSINE_UNROLLED_COMPONENTS);
    let mut chunks_rhs = rhs.chunks_exact(F64X4_COSINE_UNROLLED_COMPONENTS);
    let mut lhs_norm0 = f64x4::ZERO;
    let mut lhs_norm1 = f64x4::ZERO;
    let mut rhs_norm0 = f64x4::ZERO;
    let mut rhs_norm1 = f64x4::ZERO;
    let mut dot0 = f64x4::ZERO;
    let mut dot1 = f64x4::ZERO;
    for (lhs, rhs) in chunks_lhs.by_ref().zip(chunks_rhs.by_ref()) {
        add_cosine_components(
            &mut lhs_norm0,
            &mut rhs_norm0,
            &mut dot0,
            &lhs[0..4],
            &rhs[0..4],
        );
        add_cosine_components(
            &mut lhs_norm1,
            &mut rhs_norm1,
            &mut dot1,
            &lhs[4..8],
            &rhs[4..8],
        );
    }

    let mut chunks_lhs = chunks_lhs.remainder().chunks_exact(F64X4_COMPONENTS);
    let mut chunks_rhs = chunks_rhs.remainder().chunks_exact(F64X4_COMPONENTS);
    let mut lhs_norm_tail = f64x4::ZERO;
    let mut rhs_norm_tail = f64x4::ZERO;
    let mut dot_tail = f64x4::ZERO;
    for (lhs, rhs) in chunks_lhs.by_ref().zip(chunks_rhs.by_ref()) {
        add_cosine_components(
            &mut lhs_norm_tail,
            &mut rhs_norm_tail,
            &mut dot_tail,
            lhs,
            rhs,
        );
    }

    let mut lhs_norm = reduce_two(lhs_norm0, lhs_norm1) + lhs_norm_tail.reduce_add();
    let mut rhs_norm = reduce_two(rhs_norm0, rhs_norm1) + rhs_norm_tail.reduce_add();
    let mut dot = reduce_two(dot0, dot1) + dot_tail.reduce_add();
    for (&lhs, &rhs) in chunks_lhs.remainder().iter().zip(chunks_rhs.remainder()) {
        let lhs = f64::from(lhs);
        let rhs = f64::from(rhs);
        lhs_norm += lhs * lhs;
        rhs_norm += rhs * rhs;
        dot += lhs * rhs;
    }
    (lhs_norm, rhs_norm, dot)
}

fn cosine_components_single_chain(lhs: &[f32], rhs: &[f32]) -> (f64, f64, f64) {
    let mut chunks_lhs = lhs.chunks_exact(F64X4_COMPONENTS);
    let mut chunks_rhs = rhs.chunks_exact(F64X4_COMPONENTS);
    let mut lhs_norm = f64x4::ZERO;
    let mut rhs_norm = f64x4::ZERO;
    let mut dot = f64x4::ZERO;
    for (lhs, rhs) in chunks_lhs.by_ref().zip(chunks_rhs.by_ref()) {
        add_cosine_components(&mut lhs_norm, &mut rhs_norm, &mut dot, lhs, rhs);
    }
    let mut lhs_norm = lhs_norm.reduce_add();
    let mut rhs_norm = rhs_norm.reduce_add();
    let mut dot = dot.reduce_add();
    for (&lhs, &rhs) in chunks_lhs.remainder().iter().zip(chunks_rhs.remainder()) {
        let lhs = f64::from(lhs);
        let rhs = f64::from(rhs);
        lhs_norm += lhs * lhs;
        rhs_norm += rhs * rhs;
        dot += lhs * rhs;
    }
    (lhs_norm, rhs_norm, dot)
}

fn norm_and_dot(lhs: &[f32], rhs: &[f32]) -> (f64, f64) {
    let mut chunks_lhs = lhs.chunks_exact(F64X4_COSINE_UNROLLED_COMPONENTS);
    let mut chunks_rhs = rhs.chunks_exact(F64X4_COSINE_UNROLLED_COMPONENTS);
    let mut rhs_norm0 = f64x4::ZERO;
    let mut rhs_norm1 = f64x4::ZERO;
    let mut dot0 = f64x4::ZERO;
    let mut dot1 = f64x4::ZERO;
    for (lhs, rhs) in chunks_lhs.by_ref().zip(chunks_rhs.by_ref()) {
        add_norm_and_dot(&mut rhs_norm0, &mut dot0, &lhs[0..4], &rhs[0..4]);
        add_norm_and_dot(&mut rhs_norm1, &mut dot1, &lhs[4..8], &rhs[4..8]);
    }

    let mut chunks_lhs = chunks_lhs.remainder().chunks_exact(F64X4_COMPONENTS);
    let mut chunks_rhs = chunks_rhs.remainder().chunks_exact(F64X4_COMPONENTS);
    let mut rhs_norm_tail = f64x4::ZERO;
    let mut dot_tail = f64x4::ZERO;
    for (lhs, rhs) in chunks_lhs.by_ref().zip(chunks_rhs.by_ref()) {
        add_norm_and_dot(&mut rhs_norm_tail, &mut dot_tail, lhs, rhs);
    }

    let mut rhs_norm = reduce_two(rhs_norm0, rhs_norm1) + rhs_norm_tail.reduce_add();
    let mut dot = reduce_two(dot0, dot1) + dot_tail.reduce_add();
    for (&lhs, &rhs) in chunks_lhs.remainder().iter().zip(chunks_rhs.remainder()) {
        let lhs = f64::from(lhs);
        let rhs = f64::from(rhs);
        rhs_norm += rhs * rhs;
        dot += lhs * rhs;
    }
    (rhs_norm, dot)
}

pub(super) fn dot(lhs: &[f32], rhs: &[f32]) -> f64 {
    let mut chunks_lhs = lhs.chunks_exact(F64X4_UNROLLED_COMPONENTS);
    let mut chunks_rhs = rhs.chunks_exact(F64X4_UNROLLED_COMPONENTS);
    let mut product0 = f64x4::ZERO;
    let mut product1 = f64x4::ZERO;
    let mut product2 = f64x4::ZERO;
    let mut product3 = f64x4::ZERO;
    for (lhs, rhs) in chunks_lhs.by_ref().zip(chunks_rhs.by_ref()) {
        add_dot(&mut product0, &lhs[0..4], &rhs[0..4]);
        add_dot(&mut product1, &lhs[4..8], &rhs[4..8]);
        add_dot(&mut product2, &lhs[8..12], &rhs[8..12]);
        add_dot(&mut product3, &lhs[12..16], &rhs[12..16]);
    }

    let mut chunks_lhs = chunks_lhs.remainder().chunks_exact(F64X4_COMPONENTS);
    let mut chunks_rhs = chunks_rhs.remainder().chunks_exact(F64X4_COMPONENTS);
    let mut product_tail = f64x4::ZERO;
    for (lhs, rhs) in chunks_lhs.by_ref().zip(chunks_rhs.by_ref()) {
        add_dot(&mut product_tail, lhs, rhs);
    }

    let mut product =
        reduce_four(product0, product1, product2, product3) + product_tail.reduce_add();
    for (&lhs, &rhs) in chunks_lhs.remainder().iter().zip(chunks_rhs.remainder()) {
        product += f64::from(lhs) * f64::from(rhs);
    }
    product
}

#[inline(always)]
fn add_squared_euclidean(accumulator: &mut f64x4, lhs: &[f32], rhs: &[f32]) {
    let lhs = f64x4_from_f32(lhs);
    let rhs = f64x4_from_f32(rhs);
    let delta = lhs - rhs;
    *accumulator += delta * delta;
}

#[inline(always)]
fn add_cosine_components(
    lhs_norm: &mut f64x4,
    rhs_norm: &mut f64x4,
    dot: &mut f64x4,
    lhs: &[f32],
    rhs: &[f32],
) {
    let lhs = f64x4_from_f32(lhs);
    let rhs = f64x4_from_f32(rhs);
    *lhs_norm += lhs * lhs;
    *rhs_norm += rhs * rhs;
    *dot += lhs * rhs;
}

#[inline(always)]
fn add_norm_and_dot(rhs_norm: &mut f64x4, dot: &mut f64x4, lhs: &[f32], rhs: &[f32]) {
    let lhs = f64x4_from_f32(lhs);
    let rhs = f64x4_from_f32(rhs);
    *rhs_norm += rhs * rhs;
    *dot += lhs * rhs;
}

#[inline(always)]
fn add_dot(accumulator: &mut f64x4, lhs: &[f32], rhs: &[f32]) {
    let lhs = f64x4_from_f32(lhs);
    let rhs = f64x4_from_f32(rhs);
    *accumulator += lhs * rhs;
}

#[inline(always)]
fn reduce_four(a: f64x4, b: f64x4, c: f64x4, d: f64x4) -> f64 {
    (a + b + c + d).reduce_add()
}

#[inline(always)]
fn reduce_two(a: f64x4, b: f64x4) -> f64 {
    (a + b).reduce_add()
}

#[inline(always)]
fn f64x4_from_f32(chunk: &[f32]) -> f64x4 {
    f64x4::from([
        f64::from(chunk[0]),
        f64::from(chunk[1]),
        f64::from(chunk[2]),
        f64::from(chunk[3]),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scalar_components(lhs: &[f32], rhs: &[f32]) -> (f64, f64, f64, f64) {
        lhs.iter().zip(rhs).fold(
            (0.0, 0.0, 0.0, 0.0),
            |(distance, lhs_norm, rhs_norm, product), (&lhs, &rhs)| {
                let lhs = f64::from(lhs);
                let rhs = f64::from(rhs);
                let delta = lhs - rhs;
                (
                    distance + delta * delta,
                    lhs_norm + lhs * lhs,
                    rhs_norm + rhs * rhs,
                    product + lhs * rhs,
                )
            },
        )
    }

    fn assert_close(lhs: f64, rhs: f64) {
        assert!((lhs - rhs).abs() <= 1e-12, "{lhs} != {rhs}");
    }

    #[test]
    fn wide_metric_kernels_match_scalar_reference_for_even_and_odd_dimensions() {
        let even_lhs = [1.25, -2.0, 3.5, 4.25];
        let even_rhs = [-0.5, 2.75, 3.0, -1.25];
        let odd_lhs = [1.0, -3.0, 0.25, 7.5, -2.25];
        let odd_rhs = [4.0, -1.5, 2.0, -6.0, 0.75];

        for (lhs, rhs) in [(&even_lhs[..], &even_rhs[..]), (&odd_lhs[..], &odd_rhs[..])] {
            let (distance, lhs_norm, rhs_norm, product) = scalar_components(lhs, rhs);

            assert_close(squared_euclidean(lhs, rhs), distance);
            assert_close(dot(lhs, rhs), product);

            let (wide_lhs_norm, wide_rhs_norm, wide_product) = cosine_components(lhs, rhs);
            assert_close(wide_lhs_norm, lhs_norm);
            assert_close(wide_rhs_norm, rhs_norm);
            assert_close(wide_product, product);

            let (wide_rhs_norm, wide_product) = norm_and_dot(lhs, rhs);
            assert_close(wide_rhs_norm, rhs_norm);
            assert_close(wide_product, product);
        }
    }

    #[test]
    fn wide_metric_kernels_match_scalar_reference_for_unrolled_dimensions() {
        for dimension in [16, 17, 31, 32, 512, 513, 1024, 1025] {
            let lhs = (0..dimension)
                .map(|idx| ((idx % 17) as f32 * 0.125) - 1.0)
                .collect::<Vec<_>>();
            let rhs = (0..dimension)
                .map(|idx| 0.75 - ((idx % 13) as f32 * 0.0625))
                .collect::<Vec<_>>();
            let (distance, lhs_norm, rhs_norm, product) = scalar_components(&lhs, &rhs);

            assert_close(squared_euclidean(&lhs, &rhs), distance);
            assert_close(dot(&lhs, &rhs), product);

            let (wide_lhs_norm, wide_rhs_norm, wide_product) = cosine_components(&lhs, &rhs);
            assert_close(wide_lhs_norm, lhs_norm);
            assert_close(wide_rhs_norm, rhs_norm);
            assert_close(wide_product, product);

            let (wide_rhs_norm, wide_product) = norm_and_dot(&lhs, &rhs);
            assert_close(wide_rhs_norm, rhs_norm);
            assert_close(wide_product, product);
        }
    }
}
