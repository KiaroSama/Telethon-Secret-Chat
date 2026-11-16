//! Safe native JPEG decoding; adapted from image-rs jpeg-decoder 0.3.2.
use super::*;
impl<R: Read> Decoder<R> {
    pub(super) fn read_marker(&mut self) -> Result<Marker> {
        loop {
            // This should be an error as the JPEG spec doesn't allow extraneous data between marker segments.
            // libjpeg allows this though and there are images in the wild utilising it, so we are
            // forced to support this behavior.
            // Sony Ericsson P990i is an example of a device which produce this sort of JPEGs.
            while read_u8(&mut self.reader)? != 0xFF {}

            // Section B.1.1.2
            // All markers are assigned two-byte codes: an X’FF’ byte followed by a
            // byte which is not equal to 0 or X’FF’ (see Table B.1). Any marker may
            // optionally be preceded by any number of fill bytes, which are bytes
            // assigned code X’FF’.
            let mut byte = read_u8(&mut self.reader)?;

            // Section B.1.1.2
            // "Any marker may optionally be preceded by any number of fill bytes, which are bytes assigned code X’FF’."
            while byte == 0xFF {
                byte = read_u8(&mut self.reader)?;
            }

            if byte != 0x00 && byte != 0xFF {
                return Marker::from_u8(byte).ok_or_else(|| Error::Format("invalid marker".into()));
            }
        }
    }

    #[allow(clippy::type_complexity)]
    pub(super) fn decode_scan(
        &mut self,
        frame: &FrameInfo,
        scan: &ScanInfo,
        worker: &mut dyn Worker,
        finished: &[bool; MAX_COMPONENTS],
    ) -> Result<(Option<Marker>, Option<Vec<Vec<u8>>>)> {
        assert!(scan.component_indices.len() <= MAX_COMPONENTS);

        let components: Vec<Component> = scan
            .component_indices
            .iter()
            .map(|&i| frame.components[i].clone())
            .collect();

        // Verify that all required quantization tables has been set.
        if components
            .iter()
            .any(|component| self.quantization_tables[component.quantization_table_index].is_none())
        {
            return Err(Error::Format("use of unset quantization table".to_owned()));
        }

        if self.is_mjpeg {
            fill_default_mjpeg_tables(
                scan,
                &mut self.dc_huffman_tables,
                &mut self.ac_huffman_tables,
            );
        }

        // Verify that all required huffman tables has been set.
        if scan.spectral_selection.start == 0
            && scan
                .dc_table_indices
                .iter()
                .any(|&i| self.dc_huffman_tables[i].is_none())
        {
            return Err(Error::Format(
                "scan makes use of unset dc huffman table".to_owned(),
            ));
        }
        if scan.spectral_selection.end > 1
            && scan
                .ac_table_indices
                .iter()
                .any(|&i| self.ac_huffman_tables[i].is_none())
        {
            return Err(Error::Format(
                "scan makes use of unset ac huffman table".to_owned(),
            ));
        }

        // Prepare the worker thread for the work to come.
        for (i, component) in components.iter().enumerate() {
            if finished[i] {
                let row_data = RowData {
                    index: i,
                    component: component.clone(),
                    quantization_table: self.quantization_tables
                        [component.quantization_table_index]
                        .clone()
                        .ok_or_else(|| Error::Format("missing quantization table".into()))?,
                };

                worker.start(row_data)?;
            }
        }

        let is_progressive = frame.coding_process == CodingProcess::DctProgressive;
        let is_interleaved = components.len() > 1;
        let mut dummy_block = [0i16; 64];
        let mut huffman = HuffmanDecoder::new();
        let mut dc_predictors = [0i16; MAX_COMPONENTS];
        let mut mcus_left_until_restart = self.restart_interval;
        let mut expected_rst_num = 0;
        let mut eob_run = 0;
        let mut mcu_row_coefficients = vec![vec![]; components.len()];

        if !is_progressive {
            for (i, component) in components.iter().enumerate().filter(|&(i, _)| finished[i]) {
                let coefficients_per_mcu_row = component.block_size.width as usize
                    * component.vertical_sampling_factor as usize
                    * 64;
                mcu_row_coefficients[i] = vec![0i16; coefficients_per_mcu_row];
            }
        }

        // 4.8.2
        // When reading from the stream, if the data is non-interleaved then an MCU consists of
        // exactly one block (effectively a 1x1 sample).
        let (mcu_horizontal_samples, mcu_vertical_samples) = if is_interleaved {
            let horizontal = components
                .iter()
                .map(|component| component.horizontal_sampling_factor as u16)
                .collect::<Vec<_>>();
            let vertical = components
                .iter()
                .map(|component| component.vertical_sampling_factor as u16)
                .collect::<Vec<_>>();
            (horizontal, vertical)
        } else {
            (vec![1], vec![1])
        };

        // This also affects how many MCU values we read from stream. If it's a non-interleaved stream,
        // the MCUs will be exactly the block count.
        let (max_mcu_x, max_mcu_y) = if is_interleaved {
            (frame.mcu_size.width, frame.mcu_size.height)
        } else {
            (
                components[0].block_size.width,
                components[0].block_size.height,
            )
        };

        for mcu_y in 0..max_mcu_y {
            if mcu_y * 8 >= frame.image_size.height {
                break;
            }

            for mcu_x in 0..max_mcu_x {
                if mcu_x * 8 >= frame.image_size.width {
                    break;
                }

                if self.restart_interval > 0 {
                    if mcus_left_until_restart == 0 {
                        match huffman.take_marker(&mut self.reader)? {
                            Some(Marker::RST(n)) => {
                                if n != expected_rst_num {
                                    return Err(Error::Format(format!(
                                        "found RST{} where RST{} was expected",
                                        n, expected_rst_num
                                    )));
                                }

                                huffman.reset();
                                // Section F.2.1.3.1
                                dc_predictors = [0i16; MAX_COMPONENTS];
                                // Section G.1.2.2
                                eob_run = 0;

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

                for (i, component) in components.iter().enumerate() {
                    for v_pos in 0..mcu_vertical_samples[i] {
                        for h_pos in 0..mcu_horizontal_samples[i] {
                            let coefficients = if is_progressive {
                                let block_y = (mcu_y * mcu_vertical_samples[i] + v_pos) as usize;
                                let block_x = (mcu_x * mcu_horizontal_samples[i] + h_pos) as usize;
                                let block_offset =
                                    (block_y * component.block_size.width as usize + block_x) * 64;
                                &mut self.coefficients[scan.component_indices[i]]
                                    [block_offset..block_offset + 64]
                            } else if finished[i] {
                                // Because the worker thread operates in batches as if we were always interleaved, we
                                // need to distinguish between a single-shot buffer and one that's currently in process
                                // (for a non-interleaved) stream
                                let mcu_batch_current_row = if is_interleaved {
                                    0
                                } else {
                                    mcu_y % component.vertical_sampling_factor as u16
                                };

                                let block_y = (mcu_batch_current_row * mcu_vertical_samples[i]
                                    + v_pos) as usize;
                                let block_x = (mcu_x * mcu_horizontal_samples[i] + h_pos) as usize;
                                let block_offset =
                                    (block_y * component.block_size.width as usize + block_x) * 64;
                                &mut mcu_row_coefficients[i][block_offset..block_offset + 64]
                            } else {
                                &mut dummy_block[..64]
                            }
                            .try_into()
                            .map_err(|_| Error::Format("invalid coefficient block".into()))?;

                            if scan.successive_approximation_high == 0 {
                                decode_block(
                                    &mut self.reader,
                                    coefficients,
                                    &mut huffman,
                                    self.dc_huffman_tables[scan.dc_table_indices[i]].as_ref(),
                                    self.ac_huffman_tables[scan.ac_table_indices[i]].as_ref(),
                                    scan.spectral_selection.clone(),
                                    scan.successive_approximation_low,
                                    &mut eob_run,
                                    &mut dc_predictors[i],
                                )?;
                            } else {
                                decode_block_successive_approximation(
                                    &mut self.reader,
                                    coefficients,
                                    &mut huffman,
                                    self.ac_huffman_tables[scan.ac_table_indices[i]].as_ref(),
                                    scan.spectral_selection.clone(),
                                    scan.successive_approximation_low,
                                    &mut eob_run,
                                )?;
                            }
                        }
                    }
                }
            }

            // Send the coefficients from this MCU row to the worker thread for dequantization and idct.
            for (i, component) in components.iter().enumerate() {
                if finished[i] {
                    // In the event of non-interleaved streams, if we're still building the buffer out,
                    // keep going; don't send it yet. We also need to ensure we don't skip over the last
                    // row(s) of the image.
                    if !is_interleaved
                        && (u32::from(mcu_y) + 1) * 8 < u32::from(frame.image_size.height)
                        && (mcu_y + 1) % component.vertical_sampling_factor as u16 > 0
                    {
                        continue;
                    }

                    let coefficients_per_mcu_row = component.block_size.width as usize
                        * component.vertical_sampling_factor as usize
                        * 64;

                    let row_coefficients = if is_progressive {
                        // Because non-interleaved streams will have multiple MCU rows concatenated together,
                        // the row for calculating the offset is different.
                        let worker_mcu_y = if is_interleaved {
                            mcu_y
                        } else {
                            // Explicitly doing floor-division here
                            mcu_y / component.vertical_sampling_factor as u16
                        };

                        let offset = worker_mcu_y as usize * coefficients_per_mcu_row;
                        self.coefficients[scan.component_indices[i]]
                            [offset..offset + coefficients_per_mcu_row]
                            .to_vec()
                    } else {
                        mem::replace(
                            &mut mcu_row_coefficients[i],
                            vec![0i16; coefficients_per_mcu_row],
                        )
                    };

                    // FIXME: additional potential work stealing opportunities for rayon case if we
                    // also internally can parallelize over components.
                    worker.append_row((i, row_coefficients))?;
                }
            }
        }

        let mut marker = huffman.take_marker(&mut self.reader)?;
        while let Some(Marker::RST(_)) = marker {
            marker = self.read_marker().ok();
        }

        if finished.iter().any(|&c| c) {
            // Retrieve all the data from the worker thread.
            let mut data = vec![Vec::new(); frame.components.len()];

            for (i, &component_index) in scan.component_indices.iter().enumerate() {
                if finished[i] {
                    data[component_index] = worker.get_result(i)?;
                }
            }

            Ok((marker, Some(data)))
        } else {
            Ok((marker, None))
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn decode_block<R: Read>(
    reader: &mut R,
    coefficients: &mut [i16; 64],
    huffman: &mut HuffmanDecoder,
    dc_table: Option<&HuffmanTable>,
    ac_table: Option<&HuffmanTable>,
    spectral_selection: Range<u8>,
    successive_approximation_low: u8,
    eob_run: &mut u16,
    dc_predictor: &mut i16,
) -> Result<()> {
    debug_assert_eq!(coefficients.len(), 64);

    if spectral_selection.start == 0 {
        // Section F.2.2.1
        // Figure F.12
        let value = huffman.decode(
            reader,
            dc_table.ok_or_else(|| Error::Format("missing DC table".into()))?,
        )?;
        let diff = match value {
            0 => 0,
            1..=11 => huffman.receive_extend(reader, value)?,
            _ => {
                // Section F.1.2.1.1
                // Table F.1
                return Err(Error::Format(
                    "invalid DC difference magnitude category".to_owned(),
                ));
            }
        };

        // Malicious JPEG files can cause this add to overflow, therefore we use wrapping_add.
        // One example of such a file is tests/crashtest/images/dc-predictor-overflow.jpg
        *dc_predictor = dc_predictor.wrapping_add(diff);
        coefficients[0] = *dc_predictor << successive_approximation_low;
    }

    let mut index = cmp::max(spectral_selection.start, 1);

    if index < spectral_selection.end && *eob_run > 0 {
        *eob_run -= 1;
        return Ok(());
    }

    // Section F.1.2.2.1
    while index < spectral_selection.end {
        if let Some((value, run)) = huffman.decode_fast_ac(
            reader,
            ac_table.ok_or_else(|| Error::Format("missing AC table".into()))?,
        )? {
            index += run;

            if index >= spectral_selection.end {
                break;
            }

            coefficients[UNZIGZAG[index as usize] as usize] = value << successive_approximation_low;
            index += 1;
        } else {
            let byte = huffman.decode(
                reader,
                ac_table.ok_or_else(|| Error::Format("missing AC table".into()))?,
            )?;
            let r = byte >> 4;
            let s = byte & 0x0f;

            if s == 0 {
                match r {
                    15 => index += 16, // Run length of 16 zero coefficients.
                    _ => {
                        *eob_run = (1 << r) - 1;

                        if r > 0 {
                            *eob_run += huffman.get_bits(reader, r)?;
                        }

                        break;
                    }
                }
            } else {
                index += r;

                if index >= spectral_selection.end {
                    break;
                }

                coefficients[UNZIGZAG[index as usize] as usize] =
                    huffman.receive_extend(reader, s)? << successive_approximation_low;
                index += 1;
            }
        }
    }

    Ok(())
}

fn decode_block_successive_approximation<R: Read>(
    reader: &mut R,
    coefficients: &mut [i16; 64],
    huffman: &mut HuffmanDecoder,
    ac_table: Option<&HuffmanTable>,
    spectral_selection: Range<u8>,
    successive_approximation_low: u8,
    eob_run: &mut u16,
) -> Result<()> {
    debug_assert_eq!(coefficients.len(), 64);

    let bit = 1 << successive_approximation_low;

    if spectral_selection.start == 0 {
        // Section G.1.2.1

        if huffman.get_bits(reader, 1)? == 1 {
            coefficients[0] |= bit;
        }
    } else {
        // Section G.1.2.3

        if *eob_run > 0 {
            *eob_run -= 1;
            refine_non_zeroes(reader, coefficients, huffman, spectral_selection, 64, bit)?;
            return Ok(());
        }

        let mut index = spectral_selection.start;

        while index < spectral_selection.end {
            let byte = huffman.decode(
                reader,
                ac_table.ok_or_else(|| Error::Format("missing AC table".into()))?,
            )?;
            let r = byte >> 4;
            let s = byte & 0x0f;

            let mut zero_run_length = r;
            let mut value = 0;

            match s {
                0 => {
                    match r {
                        15 => {
                            // Run length of 16 zero coefficients.
                            // We don't need to do anything special here, zero_run_length is 15
                            // and then value (which is zero) gets written, resulting in 16
                            // zero coefficients.
                        }
                        _ => {
                            *eob_run = (1 << r) - 1;

                            if r > 0 {
                                *eob_run += huffman.get_bits(reader, r)?;
                            }

                            // Force end of block.
                            zero_run_length = 64;
                        }
                    }
                }
                1 => {
                    if huffman.get_bits(reader, 1)? == 1 {
                        value = bit;
                    } else {
                        value = -bit;
                    }
                }
                _ => return Err(Error::Format("unexpected huffman code".to_owned())),
            }

            let range = Range {
                start: index,
                end: spectral_selection.end,
            };
            index = refine_non_zeroes(reader, coefficients, huffman, range, zero_run_length, bit)?;

            if value != 0 {
                coefficients[UNZIGZAG[index as usize] as usize] = value;
            }

            index += 1;
        }
    }

    Ok(())
}

fn refine_non_zeroes<R: Read>(
    reader: &mut R,
    coefficients: &mut [i16; 64],
    huffman: &mut HuffmanDecoder,
    range: Range<u8>,
    zrl: u8,
    bit: i16,
) -> Result<u8> {
    debug_assert_eq!(coefficients.len(), 64);

    let last = range.end - 1;
    let mut zero_run_length = zrl;

    for i in range {
        let index = UNZIGZAG[i as usize] as usize;

        let coefficient = &mut coefficients[index];

        if *coefficient == 0 {
            if zero_run_length == 0 {
                return Ok(i);
            }

            zero_run_length -= 1;
        } else if huffman.get_bits(reader, 1)? == 1 && *coefficient & bit == 0 {
            if *coefficient > 0 {
                *coefficient = coefficient
                    .checked_add(bit)
                    .ok_or_else(|| Error::Format("Coefficient overflow".to_owned()))?;
            } else {
                *coefficient = coefficient
                    .checked_sub(bit)
                    .ok_or_else(|| Error::Format("Coefficient overflow".to_owned()))?;
            }
        }
    }

    Ok(last)
}
