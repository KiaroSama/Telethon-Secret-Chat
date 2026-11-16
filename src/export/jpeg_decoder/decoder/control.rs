//! Safe native JPEG decoding; adapted from image-rs jpeg-decoder 0.3.2.
use super::*;
impl<R: Read> Decoder<R> {
    pub(super) fn decode_internal(
        &mut self,
        stop_after_metadata: bool,
        worker_scope: &WorkerScope,
    ) -> Result<Vec<u8>> {
        if stop_after_metadata && self.frame.is_some() {
            // The metadata has already been read.
            return Ok(Vec::new());
        } else if self.frame.is_none()
            && (read_u8(&mut self.reader)? != 0xFF
                || Marker::from_u8(read_u8(&mut self.reader)?) != Some(Marker::SOI))
        {
            return Err(Error::Format(
                "first two bytes are not an SOI marker".to_owned(),
            ));
        }

        let mut previous_marker = Marker::SOI;
        let mut pending_marker = None;
        let mut scans_processed = 0;
        let mut planes = vec![
            Vec::<u8>::new();
            self.frame
                .as_ref()
                .map_or(0, |frame| frame.components.len())
        ];
        let mut planes_u16 = vec![
            Vec::<u16>::new();
            self.frame
                .as_ref()
                .map_or(0, |frame| frame.components.len())
        ];

        loop {
            let marker = match pending_marker.take() {
                Some(m) => m,
                None => self.read_marker()?,
            };

            match marker {
                // Frame header
                Marker::SOF(..) => {
                    // Section 4.10
                    // "An image contains only one frame in the cases of sequential and
                    //  progressive coding processes; an image contains multiple frames for the
                    //  hierarchical mode."
                    if self.frame.is_some() {
                        return Err(Error::Unsupported(UnsupportedFeature::Hierarchical));
                    }

                    let frame = parse_sof(&mut self.reader, marker)?;
                    let component_count = frame.components.len();

                    if frame.is_differential {
                        return Err(Error::Unsupported(UnsupportedFeature::Hierarchical));
                    }
                    if frame.entropy_coding == EntropyCoding::Arithmetic {
                        return Err(Error::Unsupported(
                            UnsupportedFeature::ArithmeticEntropyCoding,
                        ));
                    }
                    if frame.precision != 8 && frame.coding_process != CodingProcess::Lossless {
                        return Err(Error::Unsupported(UnsupportedFeature::SamplePrecision(
                            frame.precision,
                        )));
                    }
                    if !(2..=16).contains(&frame.precision) {
                        return Err(Error::Unsupported(UnsupportedFeature::SamplePrecision(
                            frame.precision,
                        )));
                    }
                    if component_count != 1 && component_count != 3 && component_count != 4 {
                        return Err(Error::Unsupported(UnsupportedFeature::ComponentCount(
                            component_count as u8,
                        )));
                    }

                    // Make sure we support the subsampling ratios used.
                    let _ = Upsampler::new(
                        &frame.components,
                        frame.image_size.width,
                        frame.image_size.height,
                    )?;

                    self.frame = Some(frame);

                    if stop_after_metadata {
                        return Ok(Vec::new());
                    }

                    planes = vec![Vec::new(); component_count];
                    planes_u16 = vec![Vec::new(); component_count];
                }

                // Scan header
                Marker::SOS => {
                    if self.frame.is_none() {
                        return Err(Error::Format("scan encountered before frame".to_owned()));
                    }

                    let frame = self
                        .frame
                        .clone()
                        .ok_or_else(|| Error::Format("missing frame".into()))?;
                    let scan = parse_sos(&mut self.reader, &frame)?;

                    if frame.coding_process == CodingProcess::DctProgressive
                        && self.coefficients.is_empty()
                    {
                        self.coefficients = frame
                            .components
                            .iter()
                            .map(|c| {
                                let block_count =
                                    c.block_size.width as usize * c.block_size.height as usize;
                                vec![0; block_count * 64]
                            })
                            .collect();
                    }

                    if frame.coding_process == CodingProcess::Lossless {
                        let (marker, data) = self.decode_scan_lossless(&frame, &scan)?;

                        for (i, plane) in data
                            .into_iter()
                            .enumerate()
                            .filter(|(_, plane)| !plane.is_empty())
                        {
                            planes_u16[i] = plane;
                        }
                        pending_marker = marker;
                    } else {
                        // This was previously buggy, so let's explain the log here a bit. When a
                        // progressive frame is encoded then the coefficients (DC, AC) of each
                        // component (=color plane) can be split amongst scans. In particular it can
                        // happen or at least occurs in the wild that a scan contains coefficient 0 of
                        // all components. If now one but not all components had all other coefficients
                        // delivered in previous scans then such a scan contains all components but
                        // completes only some of them! (This is technically NOT permitted for all
                        // other coefficients as the standard dictates that scans with coefficients
                        // other than the 0th must only contain ONE component so we would either
                        // complete it or not. We may want to detect and error in case more component
                        // are part of a scan than allowed.) What a weird edge case.
                        //
                        // But this means we track precisely which components get completed here.
                        let mut finished = [false; MAX_COMPONENTS];

                        if scan.successive_approximation_low == 0 {
                            for (&i, component_finished) in
                                scan.component_indices.iter().zip(&mut finished)
                            {
                                if self.coefficients_finished[i] == !0 {
                                    continue;
                                }
                                for j in scan.spectral_selection.clone() {
                                    self.coefficients_finished[i] |= 1 << j;
                                }
                                if self.coefficients_finished[i] == !0 {
                                    *component_finished = true;
                                }
                            }
                        }

                        let (marker, data) = worker_scope.get_or_init_worker(|worker| {
                            self.decode_scan(&frame, &scan, worker, &finished)
                        })?;

                        if let Some(data) = data {
                            for (i, plane) in data
                                .into_iter()
                                .enumerate()
                                .filter(|(_, plane)| !plane.is_empty())
                            {
                                if self.coefficients_finished[i] == !0 {
                                    planes[i] = plane;
                                }
                            }
                        }

                        pending_marker = marker;
                    }

                    scans_processed += 1;
                }

                // Table-specification and miscellaneous markers
                // Quantization table-specification
                Marker::DQT => {
                    let tables = parse_dqt(&mut self.reader)?;

                    for (i, &table) in tables.iter().enumerate() {
                        if let Some(table) = table {
                            let mut unzigzagged_table = [0u16; 64];

                            for j in 0..64 {
                                unzigzagged_table[UNZIGZAG[j] as usize] = table[j];
                            }

                            self.quantization_tables[i] = Some(Arc::new(unzigzagged_table));
                        }
                    }
                }
                // Huffman table-specification
                Marker::DHT => {
                    let is_baseline = self.frame.as_ref().map(|frame| frame.is_baseline);
                    let (dc_tables, ac_tables) = parse_dht(&mut self.reader, is_baseline)?;

                    let current_dc_tables = mem::take(&mut self.dc_huffman_tables);
                    self.dc_huffman_tables = dc_tables
                        .into_iter()
                        .zip(current_dc_tables)
                        .map(|(a, b)| a.or(b))
                        .collect();

                    let current_ac_tables = mem::take(&mut self.ac_huffman_tables);
                    self.ac_huffman_tables = ac_tables
                        .into_iter()
                        .zip(current_ac_tables)
                        .map(|(a, b)| a.or(b))
                        .collect();
                }
                // Arithmetic conditioning table-specification
                Marker::DAC => {
                    return Err(Error::Unsupported(
                        UnsupportedFeature::ArithmeticEntropyCoding,
                    ));
                }
                // Restart interval definition
                Marker::DRI => self.restart_interval = parse_dri(&mut self.reader)?,
                // Comment
                Marker::COM => {
                    let _comment = parse_com(&mut self.reader)?;
                }
                // Application data
                Marker::APP(..) => {
                    if let Some(data) = parse_app(&mut self.reader, marker)? {
                        match data {
                            AppData::Adobe(color_transform) => {
                                self.adobe_color_transform = Some(color_transform)
                            }
                            AppData::Jfif => {
                                // From the JFIF spec:
                                // "The APP0 marker is used to identify a JPEG FIF file.
                                //     The JPEG FIF APP0 marker is mandatory right after the SOI marker."
                                // Some JPEGs in the wild does not follow this though, so we allow
                                // JFIF headers anywhere APP0 markers are allowed.
                                /*
                                if previous_marker != Marker::SOI {
                                    return Err(Error::Format("the JFIF APP0 marker must come right after the SOI marker".to_owned()));
                                }
                                */

                                self.is_jfif = true;
                            }
                            AppData::Avi1 => self.is_mjpeg = true,
                            AppData::Icc(icc) => self.icc_markers.push(icc),
                            AppData::Exif(data) => self.exif_data = Some(data),
                            AppData::Xmp(data) => self.xmp_data = Some(data),
                            AppData::Psir(data) => self.psir_data = Some(data),
                        }
                    }
                }
                // Restart
                Marker::RST(..) => {
                    // Some encoders emit a final RST marker after entropy-coded data, which
                    // decode_scan does not take care of. So if we encounter one, we ignore it.
                    if previous_marker != Marker::SOS {
                        return Err(Error::Format(
                            "RST found outside of entropy-coded data".to_owned(),
                        ));
                    }
                }

                // Define number of lines
                Marker::DNL => {
                    // Section B.2.1
                    // "If a DNL segment (see B.2.5) is present, it shall immediately follow the first scan."
                    if previous_marker != Marker::SOS || scans_processed != 1 {
                        return Err(Error::Format(
                            "DNL is only allowed immediately after the first scan".to_owned(),
                        ));
                    }

                    return Err(Error::Unsupported(UnsupportedFeature::DNL));
                }

                // Hierarchical mode markers
                Marker::DHP | Marker::EXP => {
                    return Err(Error::Unsupported(UnsupportedFeature::Hierarchical));
                }

                // End of image
                Marker::EOI => break,

                _ => {
                    return Err(Error::Format(format!(
                        "{:?} marker found where not allowed",
                        marker
                    )));
                }
            }

            previous_marker = marker;
        }

        if self.frame.is_none() {
            return Err(Error::Format(
                "end of image encountered before frame".to_owned(),
            ));
        }

        worker_scope.get_or_init_worker(|worker| self.decode_planes(worker, planes, planes_u16))
    }
}
