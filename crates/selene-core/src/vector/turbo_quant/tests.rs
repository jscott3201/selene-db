use super::*;

#[test]
fn bit_width_validates_range() {
    assert_eq!(TurboQuantBitWidth::new(2).unwrap().levels(), 4);
    assert_eq!(TurboQuantBitWidth::new(4).unwrap().max_code(), 15);
    assert!(matches!(
        TurboQuantBitWidth::new(1),
        Err(TurboQuantCodecError::InvalidBitWidth { bits: 1 })
    ));
    assert!(matches!(
        TurboQuantBitWidth::new(5),
        Err(TurboQuantCodecError::InvalidBitWidth { bits: 5 })
    ));
}

#[test]
fn packed_codes_round_trip_all_supported_widths() {
    for bits in 2..=4 {
        let bit_width = TurboQuantBitWidth::new(bits).unwrap();
        let mut codes = TurboQuantPackedCodes::new(bit_width, 11, 3).unwrap();
        for row in 0..codes.rows() {
            for dimension in 0..codes.dimensions() {
                let code = ((row + dimension) % bit_width.levels()) as u8;
                codes.write(row, dimension, code).unwrap();
            }
        }
        for row in 0..codes.rows() {
            for dimension in 0..codes.dimensions() {
                let expected = ((row + dimension) % bit_width.levels()) as u8;
                assert_eq!(codes.read(row, dimension).unwrap(), expected);
            }
        }
    }
}

#[test]
fn three_bit_packing_crosses_byte_boundaries() {
    let bit_width = TurboQuantBitWidth::new(3).unwrap();
    let mut codes = TurboQuantPackedCodes::new(bit_width, 5, 1).unwrap();
    assert_eq!(codes.bytes_per_row(), 2);
    for (dimension, code) in [7, 0, 5, 2, 6].into_iter().enumerate() {
        codes.write(0, dimension, code).unwrap();
    }
    assert_ne!(codes.as_bytes(), &[0, 0]);
    assert_eq!(codes.read(0, 0).unwrap(), 7);
    assert_eq!(codes.read(0, 1).unwrap(), 0);
    assert_eq!(codes.read(0, 2).unwrap(), 5);
    assert_eq!(codes.read(0, 3).unwrap(), 2);
    assert_eq!(codes.read(0, 4).unwrap(), 6);
}

#[test]
fn packed_codes_validate_shape_and_bounds() {
    let bit_width = TurboQuantBitWidth::new(4).unwrap();
    assert!(matches!(
        TurboQuantPackedCodes::new(bit_width, 0, 1),
        Err(TurboQuantCodecError::InvalidDimension { dimension: 0, .. })
    ));
    assert!(matches!(
        TurboQuantPackedCodes::from_bytes(bit_width, 3, 2, vec![0; 2]),
        Err(TurboQuantCodecError::ByteLengthMismatch {
            expected: 4,
            actual: 2
        })
    ));

    let mut codes = TurboQuantPackedCodes::new(bit_width, 3, 2).unwrap();
    assert!(matches!(
        codes.read(2, 0),
        Err(TurboQuantCodecError::RowOutOfBounds { row: 2, rows: 2 })
    ));
    assert!(matches!(
        codes.read(0, 3),
        Err(TurboQuantCodecError::DimensionOutOfBounds {
            dimension: 3,
            dimensions: 3
        })
    ));
    assert!(matches!(
        codes.write(0, 0, 16),
        Err(TurboQuantCodecError::InvalidCode { code: 16, max: 15 })
    ));
}

#[test]
fn codebooks_are_sorted_and_dimension_scaled() {
    let bit_width = TurboQuantBitWidth::new(4).unwrap();
    let clipped = TurboQuantCodebook::clipped_uniform(bit_width, 128).unwrap();
    let lloyd = TurboQuantCodebook::normal_lloyd_max(bit_width, 128).unwrap();

    assert_eq!(clipped.centroids().len(), bit_width.levels());
    assert_eq!(clipped.boundaries().len(), bit_width.levels() - 1);
    assert_eq!(lloyd.centroids().len(), bit_width.levels());
    assert_eq!(lloyd.boundaries().len(), bit_width.levels() - 1);
    assert!(clipped.centroids().windows(2).all(|pair| pair[0] < pair[1]));
    assert!(lloyd.centroids().windows(2).all(|pair| pair[0] < pair[1]));
    assert!(lloyd.centroids()[0] < 0.0);
    assert!(lloyd.centroids()[bit_width.levels() - 1] > 0.0);
}

#[test]
fn codebook_encodes_and_decodes_scalars() {
    let bit_width = TurboQuantBitWidth::new(2).unwrap();
    let codebook = TurboQuantCodebook::clipped_uniform(bit_width, 64).unwrap();
    assert_eq!(
        codebook.encode_scalar(f32::NEG_INFINITY).unwrap_err(),
        TurboQuantCodecError::NonFiniteValue {
            value: f32::NEG_INFINITY,
        }
    );
    assert_eq!(codebook.encode_scalar(-100.0).unwrap(), 0);
    assert_eq!(codebook.encode_scalar(100.0).unwrap(), bit_width.max_code());
    let code = codebook.encode_scalar(0.0).unwrap();
    assert!(code <= bit_width.max_code());
    assert!(codebook.centroid(code).unwrap().is_finite());
    assert!(matches!(
        codebook.centroid(4),
        Err(TurboQuantCodecError::InvalidCode { code: 4, max: 3 })
    ));
}

#[test]
fn codebook_rejects_invalid_dimensions() {
    let bit_width = TurboQuantBitWidth::new(4).unwrap();
    assert!(matches!(
        TurboQuantCodebook::normal_lloyd_max(bit_width, 0),
        Err(TurboQuantCodecError::InvalidDimension { dimension: 0, .. })
    ));
    assert!(matches!(
        TurboQuantCodebook::normal_lloyd_max(bit_width, MAX_VECTOR_DIMENSION + 1),
        Err(TurboQuantCodecError::InvalidDimension {
            dimension,
            max: MAX_VECTOR_DIMENSION
        }) if dimension == MAX_VECTOR_DIMENSION + 1
    ));
}
