use rayon::prelude::*;

#[cfg(not(test))]
const TURBO_QUANT_PARALLEL_CALIBRATION_MIN_DIMENSION: usize = 512;
#[cfg(test)]
const TURBO_QUANT_PARALLEL_CALIBRATION_MIN_DIMENSION: usize = 4;
#[cfg(not(test))]
const TURBO_QUANT_PARALLEL_CALIBRATION_MIN_VALUES: usize = 1_000_000;
#[cfg(test)]
const TURBO_QUANT_PARALLEL_CALIBRATION_MIN_VALUES: usize = 32;
const QUANTILE_LOW_Z: f32 = -1.644_853_6;

pub(super) fn quantile_calibration(rotated: &[f32], dimension: usize) -> (Vec<f32>, Vec<f32>) {
    let rows = rotated.len() / dimension;
    let target_low = QUANTILE_LOW_Z / (dimension as f32).sqrt();
    let target_high = -target_low;
    let target_span = target_high - target_low;
    let low_index = ((rows as f64) * 0.05) as usize;
    let high_index = (((rows as f64) * 0.95) as usize).min(rows.saturating_sub(1));
    if should_parallelize_calibration(rows, dimension) {
        return quantile_calibration_parallel(
            rotated,
            dimension,
            target_low,
            target_span,
            low_index,
            high_index,
        );
    }
    quantile_calibration_sequential(
        rotated,
        dimension,
        target_low,
        target_span,
        low_index,
        high_index,
    )
}

fn should_parallelize_calibration(rows: usize, dimension: usize) -> bool {
    dimension >= TURBO_QUANT_PARALLEL_CALIBRATION_MIN_DIMENSION
        && rows.saturating_mul(dimension) >= TURBO_QUANT_PARALLEL_CALIBRATION_MIN_VALUES
}

fn quantile_calibration_sequential(
    rotated: &[f32],
    dimension: usize,
    target_low: f32,
    target_span: f32,
    low_index: usize,
    high_index: usize,
) -> (Vec<f32>, Vec<f32>) {
    let rows = rotated.len() / dimension;
    let mut shift = vec![0.0; dimension];
    let mut scale = vec![1.0; dimension];
    let mut coordinate = vec![0.0; rows];

    for dim in 0..dimension {
        for row in 0..rows {
            coordinate[row] = rotated[row * dimension + dim];
        }
        let (coordinate_shift, coordinate_scale) = coordinate_calibration(
            &mut coordinate,
            target_low,
            target_span,
            low_index,
            high_index,
        );
        shift[dim] = coordinate_shift;
        scale[dim] = coordinate_scale;
    }
    (shift, scale)
}

fn quantile_calibration_parallel(
    rotated: &[f32],
    dimension: usize,
    target_low: f32,
    target_span: f32,
    low_index: usize,
    high_index: usize,
) -> (Vec<f32>, Vec<f32>) {
    let rows = rotated.len() / dimension;
    let calibration = (0..dimension)
        .into_par_iter()
        .map_init(
            || vec![0.0; rows],
            |coordinate, dim| {
                for row in 0..rows {
                    coordinate[row] = rotated[row * dimension + dim];
                }
                coordinate_calibration(coordinate, target_low, target_span, low_index, high_index)
            },
        )
        .collect::<Vec<_>>();
    let mut shift = Vec::with_capacity(dimension);
    let mut scale = Vec::with_capacity(dimension);
    for (coordinate_shift, coordinate_scale) in calibration {
        shift.push(coordinate_shift);
        scale.push(coordinate_scale);
    }
    (shift, scale)
}

fn coordinate_calibration(
    coordinate: &mut [f32],
    target_low: f32,
    target_span: f32,
    low_index: usize,
    high_index: usize,
) -> (f32, f32) {
    let (source_low, source_high) = coordinate_quantiles(coordinate, low_index, high_index);
    let source_span = source_high - source_low;
    if source_span > 1e-6 {
        let scale = target_span / source_span;
        let shift = target_low / scale - source_low;
        (shift, scale)
    } else {
        (0.0, 1.0)
    }
}

fn coordinate_quantiles(coordinate: &mut [f32], low_index: usize, high_index: usize) -> (f32, f32) {
    debug_assert!(!coordinate.is_empty());
    debug_assert!(low_index <= high_index);
    debug_assert!(high_index < coordinate.len());

    let (_, low_value, greater) = coordinate.select_nth_unstable_by(low_index, f32::total_cmp);
    let source_low = *low_value;
    if low_index == high_index {
        return (source_low, source_low);
    }
    let high_offset = high_index - low_index - 1;
    let (_, high_value, _) = greater.select_nth_unstable_by(high_offset, f32::total_cmp);
    (source_low, *high_value)
}
