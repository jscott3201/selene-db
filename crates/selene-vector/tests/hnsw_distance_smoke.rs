//! Integration smoke tests for the BRIEF-58 distance kernels.

use selene_vector::DistanceMetric;
use selene_vector::distance::{
    cosine_similarity, distance, dot_product, is_unit_length, l2_squared,
};

#[test]
fn dot_product_orthonormal_basis() {
    let a = basis(16, 0);
    let b = basis(16, 1);

    assert_eq!(dot_product(&a, &b), 0.0);
    assert_eq!(dot_product(&a, &a), 1.0);
}

#[test]
fn l2_squared_zero_for_identical() {
    for v in [
        vec![0.0; 8],
        vec![1.0, -2.0, 3.0, -4.0],
        (0..17).map(|i| i as f32 * 0.25).collect(),
    ] {
        assert_eq!(l2_squared(&v, &v), 0.0);
    }
}

#[test]
fn l2_squared_matches_naive() {
    let a = [
        0.0, 1.0, 2.0, 3.5, -4.0, 8.0, 13.0, 21.0, -34.0, 55.0, 89.0, -144.0,
    ];
    let b = [
        1.0, -1.0, 3.0, 2.5, -8.0, 5.0, 13.0, -21.0, 34.0, 55.0, -89.0, 144.0,
    ];

    assert_close(
        l2_squared(&a, &b),
        naive_l2_squared(&a, &b),
        f32::EPSILON * 12.0,
    );
}

#[test]
fn cosine_similarity_zero_magnitude() {
    assert_eq!(cosine_similarity(&[0.0; 8], &[1.0; 8]), 0.0);
}

#[test]
fn cosine_similarity_pre_normalized_equals_dot() {
    let a = [1.0, 0.0, 0.0, 0.0];
    let b = [0.0, 1.0, 0.0, 0.0];

    assert!(is_unit_length(&a));
    assert!(is_unit_length(&b));
    assert_close(cosine_similarity(&a, &b), dot_product(&a, &b), 1e-6);
}

#[test]
fn is_unit_length_boundary() {
    assert!(is_unit_length(&[1.0, 0.0]));
    assert!(is_unit_length(&[1.0 + 1e-6, 0.0]));
    assert!(is_unit_length(&[1.0 - 1e-6, 0.0]));
    assert!(!is_unit_length(&[1.0 + 1e-4, 0.0]));
}

#[test]
fn distance_dispatcher_smaller_is_closer() {
    let same = [1.0, 0.0, 0.0, 0.0];
    let orthogonal = [0.0, 1.0, 0.0, 0.0];
    let opposite = [-1.0, 0.0, 0.0, 0.0];

    assert!(
        distance(DistanceMetric::Cosine, &same, &same)
            < distance(DistanceMetric::Cosine, &same, &orthogonal)
    );
    assert!(
        distance(DistanceMetric::L2, &same, &same)
            < distance(DistanceMetric::L2, &same, &orthogonal)
    );
    assert!(
        distance(DistanceMetric::Dot, &same, &same)
            < distance(DistanceMetric::Dot, &same, &opposite)
    );
}

#[test]
fn distance_chunk_remainder_correctness() {
    let a = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0];
    let b = [0.25; 11];

    assert_close(dot_product(&a, &b), naive_dot(&a, &b), f32::EPSILON * 11.0);
}

#[test]
fn dot_product_dimensions_match_naive_for_brief_sizes() {
    for dim in [1, 7, 8, 9, 16, 32, 100, 384, 1_536] {
        let a = vec![1.0; dim];
        let b = vec![0.25; dim];
        assert_close(
            dot_product(&a, &b),
            naive_dot(&a, &b),
            f32::EPSILON * dim as f32,
        );
    }
}

#[test]
fn distance_identical_vectors_are_zero_for_cosine_and_l2() {
    for v in [
        vec![1.0, 0.0, 0.0, 0.0],
        vec![0.5; 16],
        (0..32).map(|i| i as f32 * 0.125).collect(),
    ] {
        assert_close(distance(DistanceMetric::Cosine, &v, &v), 0.0, 1e-5);
        assert_eq!(distance(DistanceMetric::L2, &v, &v), 0.0);
    }
}

#[test]
fn cosine_parity_identical_nonzero() {
    assert_cosine_contract(&[1.0, 2.0, 3.0, 4.0], &[1.0, 2.0, 3.0, 4.0]);
}

#[test]
fn cosine_parity_orthogonal_nonzero() {
    assert_cosine_contract(&[1.0, 0.0, 0.0, 0.0], &[0.0, 1.0, 0.0, 0.0]);
}

#[test]
fn cosine_parity_zero_nonzero() {
    assert_cosine_contract(&[0.0; 4], &[1.0, 2.0, 3.0, 4.0]);
}

#[test]
fn cosine_parity_zero_zero() {
    assert_cosine_contract(&[0.0; 4], &[0.0; 4]);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "dot product dimension mismatch")]
fn debug_assert_dim_mismatch_panics() {
    let _ = dot_product(&[1.0; 8], &[1.0; 7]);
}

fn basis(dim: usize, index: usize) -> Vec<f32> {
    let mut out = vec![0.0; dim];
    out[index] = 1.0;
    out
}

fn assert_cosine_contract(a: &[f32], b: &[f32]) {
    assert_close(cosine_similarity(a, b), naive_cosine(a, b), 1e-6);
}

fn naive_dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn naive_l2_squared(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(x, y)| {
            let delta = x - y;
            delta * delta
        })
        .sum()
}

fn naive_cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot = naive_dot(a, b);
    let mag_a = naive_dot(a, a).sqrt();
    let mag_b = naive_dot(b, b).sqrt();
    if mag_a == 0.0 || mag_b == 0.0 {
        0.0
    } else {
        dot / (mag_a * mag_b)
    }
}

fn assert_close(actual: f32, expected: f32, tolerance: f32) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "actual={actual}, expected={expected}, tolerance={tolerance}"
    );
}
