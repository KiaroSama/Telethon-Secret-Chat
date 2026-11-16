//! Safe native JPEG decoding; adapted from image-rs jpeg-decoder 0.3.2.
use super::*;
impl<R: Read> Decoder<R> {
    pub(super) fn decode_planes(
        &mut self,
        worker: &mut dyn Worker,
        mut planes: Vec<Vec<u8>>,
        planes_u16: Vec<Vec<u16>>,
    ) -> Result<Vec<u8>> {
        if self.frame.is_none() {
            return Err(Error::Format(
                "end of image encountered before frame".to_owned(),
            ));
        }

        let frame = self
            .frame
            .as_ref()
            .ok_or_else(|| Error::Format("missing frame".into()))?;

        if frame
            .components
            .len()
            .checked_mul(frame.output_size.width.into())
            .and_then(|m| m.checked_mul(frame.output_size.height.into()))
            .and_then(|m| m.checked_mul(if frame.precision > 8 { 2 } else { 1 }))
            .is_none_or(|m| self.decoding_buffer_size_limit < m)
        {
            return Err(Error::Format(
                "size of decoded image exceeds maximum allowed size".to_owned(),
            ));
        }

        // If we're decoding a progressive jpeg and a component is unfinished, render what we've got
        if frame.coding_process == CodingProcess::DctProgressive
            && self.coefficients.len() == frame.components.len()
        {
            for (i, component) in frame.components.iter().enumerate() {
                // Only dealing with unfinished components
                if self.coefficients_finished[i] == !0 {
                    continue;
                }

                let quantization_table =
                    match self.quantization_tables[component.quantization_table_index].clone() {
                        Some(quantization_table) => quantization_table,
                        None => continue,
                    };

                // Get the worker prepared
                let row_data = RowData {
                    index: i,
                    component: component.clone(),
                    quantization_table,
                };
                worker.start(row_data)?;

                // Send the rows over to the worker and collect the result
                let coefficients_per_mcu_row = usize::from(component.block_size.width)
                    * usize::from(component.vertical_sampling_factor)
                    * 64;

                let mut tasks = (0..frame.mcu_size.height).map(|mcu_y| {
                    let offset = usize::from(mcu_y) * coefficients_per_mcu_row;
                    let row_coefficients =
                        self.coefficients[i][offset..offset + coefficients_per_mcu_row].to_vec();
                    (i, row_coefficients)
                });

                // FIXME: additional potential work stealing opportunities for rayon case if we
                // also internally can parallelize over components.
                worker.append_rows(&mut tasks)?;
                planes[i] = worker.get_result(i)?;
            }
        }

        if frame.coding_process == CodingProcess::Lossless {
            compute_image_lossless(frame, planes_u16)
        } else {
            compute_image(
                &frame.components,
                planes,
                frame.output_size,
                self.determine_color_transform(),
            )
        }
    }

    pub(super) fn determine_color_transform(&self) -> ColorTransform {
        if let Some(color_transform) = self.color_transform {
            return color_transform;
        }

        let Some(frame) = self.frame.as_ref() else {
            return ColorTransform::Unknown;
        };

        if frame.components.len() == 1 {
            return ColorTransform::Grayscale;
        }

        // Using logic for determining colour as described here: https://entropymine.wordpress.com/2018/10/22/how-is-a-jpeg-images-color-type-determined/

        if frame.components.len() == 3 {
            match (
                frame.components[0].identifier,
                frame.components[1].identifier,
                frame.components[2].identifier,
            ) {
                (1, 2, 3) => {
                    return ColorTransform::YCbCr;
                }
                (1, 34, 35) => {
                    return ColorTransform::JcsBgYcc;
                }
                (82, 71, 66) => {
                    return ColorTransform::RGB;
                }
                (114, 103, 98) => {
                    return ColorTransform::JcsBgRgb;
                }
                _ => {}
            }

            if self.is_jfif {
                return ColorTransform::YCbCr;
            }
        }

        if let Some(colour_transform) = self.adobe_color_transform {
            match colour_transform {
                AdobeColorTransform::Unknown => {
                    if frame.components.len() == 3 {
                        return ColorTransform::RGB;
                    } else if frame.components.len() == 4 {
                        return ColorTransform::CMYK;
                    }
                }
                AdobeColorTransform::YCbCr => {
                    return ColorTransform::YCbCr;
                }
                AdobeColorTransform::YCCK => {
                    return ColorTransform::YCCK;
                }
            }
        } else if frame.components.len() == 4 {
            return ColorTransform::CMYK;
        }

        if frame.components.len() == 4 {
            ColorTransform::YCCK
        } else if frame.components.len() == 3 {
            ColorTransform::YCbCr
        } else {
            ColorTransform::Unknown
        }
    }
}

pub(super) fn compute_image(
    components: &[Component],
    mut data: Vec<Vec<u8>>,
    output_size: Dimensions,
    color_transform: ColorTransform,
) -> Result<Vec<u8>> {
    if data.is_empty() || data.iter().any(Vec::is_empty) {
        return Err(Error::Format("not all components have data".to_owned()));
    }

    if components.len() == 1 {
        let component = &components[0];
        let mut decoded: Vec<u8> = data.remove(0);

        let width = component.size.width as usize;
        let height = component.size.height as usize;
        let size = width * height;
        let line_stride = component.block_size.width as usize * component.dct_scale;

        // if the image width is a multiple of the block size,
        // then we don't have to move bytes in the decoded data
        if usize::from(output_size.width) != line_stride {
            // The first line already starts at index 0, so we need to move only lines 1..height
            // We move from the top down because all lines are being moved backwards.
            for y in 1..height {
                let destination_idx = y * width;
                let source_idx = y * line_stride;
                let end = source_idx + width;
                decoded.copy_within(source_idx..end, destination_idx);
            }
        }
        decoded.resize(size, 0);
        Ok(decoded)
    } else {
        compute_image_portable(components, data, output_size, color_transform)
    }
}

#[allow(clippy::type_complexity)]
pub(crate) fn choose_color_convert_func(
    component_count: usize,
    color_transform: ColorTransform,
) -> Result<fn(&[Vec<u8>], &mut [u8])> {
    match component_count {
        3 => match color_transform {
            ColorTransform::None => Ok(color_no_convert),
            ColorTransform::Grayscale => Err(Error::Format(
                "Invalid number of channels (3) for Grayscale data".to_string(),
            )),
            ColorTransform::RGB => Ok(color_convert_line_rgb),
            ColorTransform::YCbCr => Ok(color_convert_line_ycbcr),
            ColorTransform::CMYK => Err(Error::Format(
                "Invalid number of channels (3) for CMYK data".to_string(),
            )),
            ColorTransform::YCCK => Err(Error::Format(
                "Invalid number of channels (3) for YCCK data".to_string(),
            )),
            ColorTransform::JcsBgYcc => Err(Error::Unsupported(
                UnsupportedFeature::ColorTransform(ColorTransform::JcsBgYcc),
            )),
            ColorTransform::JcsBgRgb => Err(Error::Unsupported(
                UnsupportedFeature::ColorTransform(ColorTransform::JcsBgRgb),
            )),
            ColorTransform::Unknown => Err(Error::Format("Unknown colour transform".to_string())),
        },
        4 => match color_transform {
            ColorTransform::None => Ok(color_no_convert),
            ColorTransform::Grayscale => Err(Error::Format(
                "Invalid number of channels (4) for Grayscale data".to_string(),
            )),
            ColorTransform::RGB => Err(Error::Format(
                "Invalid number of channels (4) for RGB data".to_string(),
            )),
            ColorTransform::YCbCr => Err(Error::Format(
                "Invalid number of channels (4) for YCbCr data".to_string(),
            )),
            ColorTransform::CMYK => Ok(color_convert_line_cmyk),
            ColorTransform::YCCK => Ok(color_convert_line_ycck),

            ColorTransform::JcsBgYcc => Err(Error::Unsupported(
                UnsupportedFeature::ColorTransform(ColorTransform::JcsBgYcc),
            )),
            ColorTransform::JcsBgRgb => Err(Error::Unsupported(
                UnsupportedFeature::ColorTransform(ColorTransform::JcsBgRgb),
            )),
            ColorTransform::Unknown => Err(Error::Format("Unknown colour transform".to_string())),
        },
        _ => panic!(),
    }
}

fn color_convert_line_rgb(data: &[Vec<u8>], output: &mut [u8]) {
    assert!(data.len() == 3, "wrong number of components for rgb");
    let Ok([r, g, b]): std::result::Result<&[Vec<u8>; 3], _> = data.try_into() else {
        return;
    };
    for (((chunk, r), g), b) in output
        .as_chunks_mut::<3>()
        .0
        .iter_mut()
        .zip(r.iter())
        .zip(g.iter())
        .zip(b.iter())
    {
        chunk[0] = *r;
        chunk[1] = *g;
        chunk[2] = *b;
    }
}

fn color_convert_line_ycbcr(data: &[Vec<u8>], output: &mut [u8]) {
    assert!(data.len() == 3, "wrong number of components for ycbcr");
    let Ok([y, cb, cr]): std::result::Result<&[Vec<u8>; 3], _> = data.try_into() else {
        return;
    };

    for (((chunk, y), cb), cr) in output
        .as_chunks_mut::<3>()
        .0
        .iter_mut()
        .zip(y.iter())
        .zip(cb.iter())
        .zip(cr.iter())
    {
        let (r, g, b) = ycbcr_to_rgb(*y, *cb, *cr);
        chunk[0] = r;
        chunk[1] = g;
        chunk[2] = b;
    }
}

fn color_convert_line_ycck(data: &[Vec<u8>], output: &mut [u8]) {
    assert!(data.len() == 4, "wrong number of components for ycck");
    let Ok([c, m, y, k]): std::result::Result<&[Vec<u8>; 4], _> = data.try_into() else {
        return;
    };

    for ((((chunk, c), m), y), k) in output
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(c.iter())
        .zip(m.iter())
        .zip(y.iter())
        .zip(k.iter())
    {
        let (r, g, b) = ycbcr_to_rgb(*c, *m, *y);
        chunk[0] = r;
        chunk[1] = g;
        chunk[2] = b;
        chunk[3] = 255 - *k;
    }
}

fn color_convert_line_cmyk(data: &[Vec<u8>], output: &mut [u8]) {
    assert!(data.len() == 4, "wrong number of components for cmyk");
    let Ok([c, m, y, k]): std::result::Result<&[Vec<u8>; 4], _> = data.try_into() else {
        return;
    };

    for ((((chunk, c), m), y), k) in output
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(c.iter())
        .zip(m.iter())
        .zip(y.iter())
        .zip(k.iter())
    {
        chunk[0] = 255 - c;
        chunk[1] = 255 - m;
        chunk[2] = 255 - y;
        chunk[3] = 255 - k;
    }
}

fn color_no_convert(data: &[Vec<u8>], output: &mut [u8]) {
    for (pixel, chunk) in output.chunks_mut(data.len()).enumerate() {
        for (sample, channel) in chunk.iter_mut().zip(data) {
            *sample = channel[pixel];
        }
    }
}

// libjpeg-turbo's 16-bit color tables (jccolor/jdcolor), not stb's 20-bit approximation.
fn ycbcr_to_rgb(y: u8, cb: u8, cr: u8) -> (u8, u8, u8) {
    let y = i32::from(y);
    let cb = i32::from(cb) - 128;
    let cr = i32::from(cr) - 128;
    let clamp = |n: i32| n.clamp(0, 255) as u8;
    (
        clamp(y + ((91881 * cr + 32768) >> 16)),
        clamp(y + ((-22554 * cb - 46802 * cr + 32768) >> 16)),
        clamp(y + ((116130 * cb + 32768) >> 16)),
    )
}
#[cfg(test)]
pub(super) fn audit_no_transform() {
    let mut output = [0; 6];
    color_no_convert(&[vec![1, 2], vec![3, 4], vec![5, 6]], &mut output);
    assert_eq!(output, [1, 3, 5, 2, 4, 6]);
}
