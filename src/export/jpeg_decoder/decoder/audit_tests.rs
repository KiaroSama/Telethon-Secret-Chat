//! Public, synthetic JPEG frames for regression assertions in the existing codec case.
#![allow(clippy::unwrap_used)]
use super::*;
use std::io::Cursor;
fn segment(out: &mut Vec<u8>, marker: u8, data: &[u8]) {
    out.extend([255, marker]);
    out.extend(((data.len() + 2) as u16).to_be_bytes());
    out.extend(data);
}
fn header(marker: u8, precision: u8, width: u16, components: u8) -> Vec<u8> {
    let mut out = vec![255, 216];
    let mut sof = vec![precision, 0, 1];
    sof.extend(width.to_be_bytes());
    sof.push(components);
    for id in 1..=components {
        sof.extend([id, 0x11, 0]);
    }
    segment(&mut out, marker, &sof);
    let mut table = vec![0, 1, 1];
    table.extend([0; 14]);
    table.extend([0, 1]);
    segment(&mut out, 0xc4, &table);
    out
}
fn lossless(
    precision: u8,
    width: u16,
    predictor: u8,
    transform: u8,
    restart: bool,
    entropy: &[u8],
) -> Vec<u8> {
    let mut out = header(0xc3, precision, width, 1);
    if restart {
        segment(&mut out, 0xdd, &[0, 2]);
    }
    segment(&mut out, 0xda, &[1, 1, 0, predictor, 0, transform]);
    out.extend(entropy);
    out.extend([255, 217]);
    out
}
fn decode(bytes: Vec<u8>) -> Vec<u8> {
    Decoder::new(Cursor::new(bytes)).decode().unwrap()
}
pub(crate) fn run() {
    let checks: &[(&str, fn())] = &[
        ("allocation_before_limit", || {
            let mut bytes = header(0xc2, 8, 16, 1);
            segment(&mut bytes, 0xda, &[1, 1, 0, 0, 0, 0]);
            bytes.extend([255, 217]);
            let mut decoder = Decoder::new(Cursor::new(bytes));
            decoder.set_max_decoding_buffer_size(16);
            assert!(decoder.decode().is_err());
            assert!(decoder.coefficients.is_empty());
        }),
        ("lossless_restart", || {
            for predictor in [1, 2] {
                assert_eq!(
                    decode(lossless(8, 4, predictor, 0, true, &[0xaf, 255, 0xd0, 0x3f])),
                    [129, 129, 128, 128]
                );
            }
            let mut bytes = lossless(8, 2, 2, 0, true, &[0xaf, 255, 0xd0, 0x3f]);
            bytes[7..9].copy_from_slice(&2u16.to_be_bytes());
            assert_eq!(decode(bytes), [129, 129, 128, 128]);
        }),
        ("lossless_point_transform", || {
            assert_eq!(decode(lossless(8, 1, 1, 7, false, &[0x7f])), [128]);
            for predictor in [1, 2] {
                assert_eq!(
                    decode(lossless(8, 2, predictor, 1, false, &[0x3f])),
                    [128, 128]
                );
            }
        }),
        ("lossless_low_precision", || {
            assert_eq!(decode(lossless(7, 1, 1, 0, false, &[0x7f])), [64]);
        }),
        ("lossless_plane_indices", || {
            let mut bytes = header(0xc3, 8, 1, 3);
            for (id, entropy) in [(1, 0xbf), (2, 0x7f), (3, 0x9f)] {
                segment(&mut bytes, 0xda, &[1, id, 0, 1, 0, 0]);
                bytes.push(entropy);
            }
            bytes.extend([255, 217]);
            assert_eq!(decode(bytes), [129, 128, 127]);
        }),
        ("lossless_output_byte_limit", || {
            let mut high_precision = header(0xc3, 16, 2, 3);
            segment(&mut high_precision, 0xda, &[3, 1, 0, 2, 0, 3, 0, 1, 0, 0]);
            high_precision.extend([0x03, 255, 217]);
            let mut decoder = Decoder::new(Cursor::new(high_precision));
            assert!(decoder.read_info().is_err());
            let mut decoder = Decoder::new(Cursor::new(lossless(16, 1, 1, 0, false, &[0x7f])));
            decoder.set_max_decoding_buffer_size(1);
            assert!(decoder.decode().is_err());
        }),
        ("no_transform_interleave", super::planes::audit_no_transform),
        ("avi1_polarities", || {
            for polarity in [0, 1, 2] {
                let data = [0, 7, b'A', b'V', b'I', b'1', polarity];
                assert!(matches!(
                    parse_app(&mut Cursor::new(data), Marker::APP(0)).unwrap(),
                    Some(AppData::Avi1)
                ));
            }
        }),
        ("progressive_al_eight", || {
            let bytes = header(0xc2, 8, 1, 1);
            let mut decoder = Decoder::new(Cursor::new(bytes));
            decoder.read_info().unwrap();
            assert!(
                parse_sos(
                    &mut Cursor::new([0, 8, 1, 1, 0, 0, 0, 8]),
                    decoder.frame.as_ref().unwrap()
                )
                .is_ok()
            );
        }),
        ("tall_grayscale_final_row", || {
            let mut bytes = header(0xc0, 8, 8, 1);
            bytes[7..9].copy_from_slice(&65535u16.to_be_bytes());
            let mut quant = vec![0];
            quant.extend([1; 64]);
            segment(&mut bytes, 0xdb, &quant);
            let mut ac = vec![0x10, 1];
            ac.extend([0; 15]);
            ac.push(0);
            segment(&mut bytes, 0xc4, &ac);
            segment(&mut bytes, 0xda, &[1, 1, 0, 0, 63, 0]);
            bytes.extend(vec![0; 2048]);
            bytes.extend([255, 217]);
            let decoded = decode(bytes);
            assert_eq!(decoded.len(), 8 * 65535);
            assert!(decoded.iter().all(|byte| *byte == 128));
        }),
        ("one_eighth_idct_rounding", || {
            let mut coefficients = [0; 64];
            coefficients[0] = 4;
            let mut pixel = [0];
            super::super::idct::dequantize_and_idct_block(
                1,
                &coefficients,
                &[1; 64],
                1,
                &mut pixel,
            );
            assert_eq!(pixel, [129]);
        }),
    ];
    let failures = checks
        .iter()
        .filter_map(|(name, check)| std::panic::catch_unwind(*check).err().map(|_| *name))
        .collect::<Vec<_>>();
    assert!(failures.is_empty(), "JPEG audit regressions: {failures:?}");
}
