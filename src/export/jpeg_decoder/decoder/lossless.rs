use super::super::decoder::{Decoder, MAX_COMPONENTS};
use super::super::error::{Error, Result};
use super::super::huffman::HuffmanDecoder;
use super::super::marker::Marker;
use super::super::parser::Predictor;
use super::super::parser::{Component, FrameInfo, ScanInfo};
use std::io::Read;

impl<R: Read> Decoder<R> {
    /// decode_scan_lossless
    pub fn decode_scan_lossless(
        &mut self,
        frame: &FrameInfo,
        scan: &ScanInfo,
    ) -> Result<(Option<Marker>, Vec<Vec<u16>>)> {
        let ncomp = scan.component_indices.len();
        let npixel = frame.image_size.height as usize * frame.image_size.width as usize;
        assert!(ncomp <= MAX_COMPONENTS);
        let mut results = vec![vec![0u16; npixel]; ncomp];

        let components: Vec<Component> = scan
            .component_indices
            .iter()
            .map(|&i| frame.components[i].clone())
            .collect();

        // Verify that all required huffman tables has been set.
        if scan
            .dc_table_indices
            .iter()
            .any(|&i| self.dc_huffman_tables[i].is_none())
        {
            return Err(Error::Format(
                "scan makes use of unset dc huffman table".to_owned(),
            ));
        }

        let mut huffman = HuffmanDecoder::new();
        let reader = &mut self.reader;
        let mut mcus_left_until_restart = self.restart_interval;
        let mut expected_rst_num = 0;

        let width = frame.image_size.width as usize;
        let height = frame.image_size.height as usize;

        let mut differences = vec![Vec::with_capacity(npixel); ncomp];
        for _mcu_y in 0..height {
            for _mcu_x in 0..width {
                if self.restart_interval > 0 {
                    if mcus_left_until_restart == 0 {
                        match huffman.take_marker(reader)? {
                            Some(Marker::RST(n)) => {
                                if n != expected_rst_num {
                                    return Err(Error::Format(format!(
                                        "found RST{} where RST{} was expected",
                                        n, expected_rst_num
                                    )));
                                }

                                huffman.reset();

                                expected_rst_num = (expected_rst_num + 1) % 8;
                                mcus_left_until_restart = self.restart_interval;
                            }
                            Some(marker) => {
                                return Err(Error::Format(format!(
                                    "found marker {:?} inside scan where RST{} was expected",
                                    marker, expected_rst_num
                                )));
                            }
                            None => {
                                return Err(Error::Format(format!(
                                    "no marker found where RST{} was expected",
                                    expected_rst_num
                                )));
                            }
                        }
                    }

                    mcus_left_until_restart -= 1;
                }

                for (i, _component) in components.iter().enumerate() {
                    let dc_table = self.dc_huffman_tables[scan.dc_table_indices[i]]
                        .as_ref()
                        .ok_or_else(|| Error::Format("missing DC table".into()))?;
                    let value = huffman.decode(reader, dc_table)?;
                    let diff = match value {
                        0 => 0,
                        1..=15 => huffman.receive_extend(reader, value)? as i32,
                        16 => 32768,
                        _ => {
                            // Section F.1.2.1.1
                            // Table F.1
                            return Err(Error::Format(
                                "invalid DC difference magnitude category".to_owned(),
                            ));
                        }
                    };
                    differences[i].push(diff);
                }
            }
        }

        // Predict in the reduced precision domain; apply the point transform once.
        for y in 0..height {
            for x in 0..width {
                let at = y * width + x;
                let restart = self.restart_interval > 0
                    && at.is_multiple_of(usize::from(self.restart_interval));
                for i in 0..ncomp {
                    let ra = if x > 0 {
                        i32::from(results[i][at - 1])
                    } else {
                        0
                    };
                    let rb = if y > 0 {
                        i32::from(results[i][at - width])
                    } else {
                        0
                    };
                    let rc = if x > 0 && y > 0 {
                        i32::from(results[i][at - width - 1])
                    } else {
                        0
                    };
                    let prediction = predict(
                        ra,
                        rb,
                        rc,
                        scan.predictor_selection,
                        scan.point_transform,
                        frame.precision,
                        x,
                        y,
                        restart,
                    );
                    results[i][at] = ((prediction + differences[i][at]) & 0xffff) as u16;
                }
            }
        }
        for plane in &mut results {
            for sample in plane {
                *sample <<= scan.point_transform;
            }
        }

        let mut marker = huffman.take_marker(&mut self.reader)?;
        while let Some(Marker::RST(_)) = marker {
            marker = self.read_marker().ok();
        }
        Ok((marker, results))
    }
}

/// H.1.2.1
#[allow(clippy::too_many_arguments)]
fn predict(
    ra: i32,
    rb: i32,
    rc: i32,
    predictor: Predictor,
    point_transform: u8,
    input_precision: u8,
    ix: usize,
    iy: usize,
    restart: bool,
) -> i32 {
    if (ix == 0 && iy == 0) || restart {
        // start of first line or restart
        if input_precision > 1 + point_transform {
            1 << (input_precision - point_transform - 1)
        } else {
            0
        }
    } else if iy == 0 {
        // rest of first line
        ra
    } else if ix == 0 {
        // start of other line
        rb
    } else {
        // use predictor Table H.1
        match predictor {
            Predictor::NoPrediction => 0,
            Predictor::Ra => ra,
            Predictor::Rb => rb,
            Predictor::Rc => rc,
            Predictor::RaRbRc1 => ra + rb - rc,
            Predictor::RaRbRc2 => ra + ((rb - rc) >> 1),
            Predictor::RaRbRc3 => rb + ((ra - rc) >> 1),
            Predictor::RaRb => (ra + rb) / 2,
        }
    }
}

pub fn compute_image_lossless(frame: &FrameInfo, mut data: Vec<Vec<u16>>) -> Result<Vec<u8>> {
    if data.is_empty() || data.iter().any(Vec::is_empty) {
        return Err(Error::Format("not all components have data".to_owned()));
    }
    let output_size = frame.output_size;
    let components = &frame.components;
    let ncomp = components.len();

    if ncomp == 1 {
        let decoded = convert_to_u8(frame, data.remove(0));
        Ok(decoded)
    } else {
        let mut decoded: Vec<u16> =
            vec![0u16; ncomp * output_size.width as usize * output_size.height as usize];
        for (x, chunk) in decoded.chunks_mut(ncomp).enumerate() {
            for (i, (component_data, _)) in data.iter().zip(components.iter()).enumerate() {
                chunk[i] = component_data[x];
            }
        }
        let decoded = convert_to_u8(frame, decoded);
        Ok(decoded)
    }
}

fn convert_to_u8(frame: &FrameInfo, data: Vec<u16>) -> Vec<u8> {
    if frame.precision <= 8 {
        data.iter().map(|x| *x as u8).collect()
    } else {
        // we output native endian, which is the standard for image-rs
        let ne_bytes: Vec<_> = data.iter().map(|x| x.to_ne_bytes()).collect();
        ne_bytes.concat()
    }
}
