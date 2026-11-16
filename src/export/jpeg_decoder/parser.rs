use super::error::{Error, Result, UnsupportedFeature};
use super::huffman::{HuffmanTable, HuffmanTableClass};
use super::marker::Marker;
use super::marker::Marker::*;
use super::{read_u8, read_u16_from_be};
use core::ops::Range;
use std::borrow::ToOwned;
use std::io::{self, Read};
use std::vec::Vec;
use std::{format, vec};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dimensions {
    pub width: u16,
    pub height: u16,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EntropyCoding {
    Huffman,
    Arithmetic,
}

/// Represents the coding process of an image.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CodingProcess {
    /// Sequential Discrete Cosine Transform
    DctSequential,
    /// Progressive Discrete Cosine Transform
    DctProgressive,
    /// Lossless
    Lossless,
}

// Table H.1
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Predictor {
    NoPrediction,
    Ra,
    Rb,
    Rc,
    RaRbRc1, // Ra + Rb - Rc
    RaRbRc2, // Ra + ((Rb - Rc) >> 1)
    RaRbRc3, // Rb + ((Ra - Rb) >> 1)
    RaRb,    // (Ra + Rb)/2
}

#[derive(Clone)]
pub struct FrameInfo {
    pub is_baseline: bool,
    pub is_differential: bool,
    pub coding_process: CodingProcess,
    pub entropy_coding: EntropyCoding,
    pub precision: u8,

    pub image_size: Dimensions,
    pub output_size: Dimensions,
    pub mcu_size: Dimensions,
    pub components: Vec<Component>,
}

#[derive(Debug)]
pub struct ScanInfo {
    pub component_indices: Vec<usize>,
    pub dc_table_indices: Vec<usize>,
    pub ac_table_indices: Vec<usize>,

    pub spectral_selection: Range<u8>,
    pub predictor_selection: Predictor, // for lossless
    pub successive_approximation_high: u8,
    pub successive_approximation_low: u8,
    pub point_transform: u8, // for lossless
}

#[derive(Clone, Debug)]
pub struct Component {
    pub identifier: u8,

    pub horizontal_sampling_factor: u8,
    pub vertical_sampling_factor: u8,

    pub quantization_table_index: usize,

    pub dct_scale: usize,

    pub size: Dimensions,
    pub block_size: Dimensions,
}

#[derive(Debug)]
pub enum AppData {
    Adobe(AdobeColorTransform),
    Jfif,
    Avi1,
    Icc(IccChunk),
    Exif(Vec<u8>),
    Xmp(Vec<u8>),
    Psir(Vec<u8>),
}

// http://www.sno.phy.queensu.ca/~phil/exiftool/TagNames/JPEG.html#Adobe
#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AdobeColorTransform {
    // RGB or CMYK
    Unknown,
    YCbCr,
    // YCbCrK
    YCCK,
}
#[derive(Debug)]
pub struct IccChunk {
    pub num_markers: u8,
    pub seq_no: u8,
    pub data: Vec<u8>,
}

impl FrameInfo {
    pub(crate) fn update_idct_size(&mut self, idct_size: usize) -> Result<()> {
        for component in &mut self.components {
            component.dct_scale = idct_size;
        }

        update_component_sizes(self.image_size, &mut self.components)?;

        self.output_size = Dimensions {
            width: (self.image_size.width as f32 * idct_size as f32 / 8.0).ceil() as u16,
            height: (self.image_size.height as f32 * idct_size as f32 / 8.0).ceil() as u16,
        };

        Ok(())
    }
}

fn read_length<R: Read>(reader: &mut R, marker: Marker) -> Result<usize> {
    assert!(marker.has_length());

    // length is including itself.
    let length = usize::from(read_u16_from_be(reader)?);

    if length < 2 {
        return Err(Error::Format(format!(
            "encountered {:?} with invalid length {}",
            marker, length
        )));
    }

    Ok(length - 2)
}

fn skip_bytes<R: Read>(reader: &mut R, length: usize) -> Result<()> {
    let length = length as u64;
    let to_skip = &mut reader.by_ref().take(length);
    let copied = io::copy(to_skip, &mut io::sink())?;
    if copied < length {
        Err(Error::Io(io::ErrorKind::UnexpectedEof.into()))
    } else {
        Ok(())
    }
}

// Section B.2.2
pub fn parse_sof<R: Read>(reader: &mut R, marker: Marker) -> Result<FrameInfo> {
    let length = read_length(reader, marker)?;

    if length <= 6 {
        return Err(Error::Format("invalid length in SOF".to_owned()));
    }

    let is_baseline = marker == SOF(0);
    let is_differential = match marker {
        SOF(0..=3) | SOF(9..=11) => false,
        SOF(5..=7) | SOF(13..=15) => true,
        _ => panic!(),
    };
    let coding_process = match marker {
        SOF(0) | SOF(1) | SOF(5) | SOF(9) | SOF(13) => CodingProcess::DctSequential,
        SOF(2) | SOF(6) | SOF(10) | SOF(14) => CodingProcess::DctProgressive,
        SOF(3) | SOF(7) | SOF(11) | SOF(15) => CodingProcess::Lossless,
        _ => panic!(),
    };
    let entropy_coding = match marker {
        SOF(0..=3) | SOF(5..=7) => EntropyCoding::Huffman,
        SOF(9..=11) | SOF(13..=15) => EntropyCoding::Arithmetic,
        _ => panic!(),
    };

    let precision = read_u8(reader)?;

    match precision {
        8 => {}
        12 => {
            if is_baseline {
                return Err(Error::Format(
                    "12 bit sample precision is not allowed in baseline".to_owned(),
                ));
            }
        }
        _ => {
            if coding_process != CodingProcess::Lossless || precision > 16 {
                return Err(Error::Format(format!(
                    "invalid precision {} in frame header",
                    precision
                )));
            }
        }
    }

    let height = read_u16_from_be(reader)?;
    let width = read_u16_from_be(reader)?;

    // height:
    // "Value 0 indicates that the number of lines shall be defined by the DNL marker and
    //     parameters at the end of the first scan (see B.2.5)."
    if height == 0 {
        return Err(Error::Unsupported(UnsupportedFeature::DNL));
    }

    if width == 0 {
        return Err(Error::Format("zero width in frame header".to_owned()));
    }

    let component_count = read_u8(reader)?;

    if component_count == 0 {
        return Err(Error::Format(
            "zero component count in frame header".to_owned(),
        ));
    }
    if coding_process == CodingProcess::DctProgressive && component_count > 4 {
        return Err(Error::Format(
            "progressive frame with more than 4 components".to_owned(),
        ));
    }

    if length != 6 + 3 * component_count as usize {
        return Err(Error::Format("invalid length in SOF".to_owned()));
    }

    let mut components: Vec<Component> = Vec::with_capacity(component_count as usize);

    for _ in 0..component_count {
        let identifier = read_u8(reader)?;

        // Each component's identifier must be unique.
        if components.iter().any(|c| c.identifier == identifier) {
            return Err(Error::Format(format!(
                "duplicate frame component identifier {}",
                identifier
            )));
        }

        let byte = read_u8(reader)?;
        let horizontal_sampling_factor = byte >> 4;
        let vertical_sampling_factor = byte & 0x0f;

        if horizontal_sampling_factor == 0 || horizontal_sampling_factor > 4 {
            return Err(Error::Format(format!(
                "invalid horizontal sampling factor {}",
                horizontal_sampling_factor
            )));
        }
        if vertical_sampling_factor == 0 || vertical_sampling_factor > 4 {
            return Err(Error::Format(format!(
                "invalid vertical sampling factor {}",
                vertical_sampling_factor
            )));
        }

        let quantization_table_index = read_u8(reader)?;

        if quantization_table_index > 3
            || (coding_process == CodingProcess::Lossless && quantization_table_index != 0)
        {
            return Err(Error::Format(format!(
                "invalid quantization table index {}",
                quantization_table_index
            )));
        }

        components.push(Component {
            identifier,
            horizontal_sampling_factor,
            vertical_sampling_factor,
            quantization_table_index: quantization_table_index as usize,
            dct_scale: 8,
            size: Dimensions {
                width: 0,
                height: 0,
            },
            block_size: Dimensions {
                width: 0,
                height: 0,
            },
        });
    }

    let mcu_size = update_component_sizes(Dimensions { width, height }, &mut components)?;

    Ok(FrameInfo {
        is_baseline,
        is_differential,
        coding_process,
        entropy_coding,
        precision,
        image_size: Dimensions { width, height },
        output_size: Dimensions { width, height },
        mcu_size,
        components,
    })
}

/// Returns ceil(x/y), requires x>0
fn ceil_div(x: u32, y: u32) -> Result<u16> {
    if x == 0 || y == 0 {
        // TODO Determine how this error is reached. Can we validate input
        // earlier and error out then?
        return Err(Error::Format("invalid dimensions".to_owned()));
    }
    Ok((1 + ((x - 1) / y)) as u16)
}

fn update_component_sizes(size: Dimensions, components: &mut [Component]) -> Result<Dimensions> {
    let h_max = components
        .iter()
        .map(|c| c.horizontal_sampling_factor)
        .max()
        .ok_or_else(|| Error::Format("missing components".into()))? as u32;
    let v_max = components
        .iter()
        .map(|c| c.vertical_sampling_factor)
        .max()
        .ok_or_else(|| Error::Format("missing components".into()))? as u32;

    let mcu_size = Dimensions {
        width: ceil_div(size.width as u32, h_max * 8)?,
        height: ceil_div(size.height as u32, v_max * 8)?,
    };

    for component in components {
        component.size.width = ceil_div(
            size.width as u32
                * component.horizontal_sampling_factor as u32
                * component.dct_scale as u32,
            h_max * 8,
        )?;
        component.size.height = ceil_div(
            size.height as u32
                * component.vertical_sampling_factor as u32
                * component.dct_scale as u32,
            v_max * 8,
        )?;

        component.block_size.width = mcu_size.width * component.horizontal_sampling_factor as u16;
        component.block_size.height = mcu_size.height * component.vertical_sampling_factor as u16;
    }

    Ok(mcu_size)
}

#[test]
#[allow(clippy::unwrap_used)]
fn desktop_thumbnail_bytes_and_component_sizes() {
    let mut components = [Component {
        identifier: 1,
        horizontal_sampling_factor: 2,
        vertical_sampling_factor: 2,
        quantization_table_index: 0,
        dct_scale: 8,
        size: Dimensions {
            width: 0,
            height: 0,
        },
        block_size: Dimensions {
            width: 0,
            height: 0,
        },
    }];
    let mcu = update_component_sizes(
        Dimensions {
            width: 800,
            height: 280,
        },
        &mut components,
    )
    .unwrap();
    assert_eq!(
        mcu,
        Dimensions {
            width: 50,
            height: 18
        }
    );
    assert_eq!(
        components[0].block_size,
        Dimensions {
            width: 100,
            height: 36
        }
    );
    assert_eq!(
        components[0].size,
        Dimensions {
            width: 800,
            height: 280
        }
    );
    // The native decoder geometry regression accompanies the independently
    // captured Desktop thumbnail oracle that exercises this actual codec.
    use crate::export::thumbnail::{ThumbnailOptions, write_image_thumb};
    use sha2::{Digest, Sha256};
    let folder = tempfile::tempdir().unwrap();
    std::fs::create_dir(folder.path().join("photos")).unwrap();
    std::fs::write(
        folder.path().join("photos/p.jpg"),
        include_bytes!("../../../tests/fixtures/tdexport_thumbs/rgb.source.jpg"),
    )
    .unwrap();
    let (path, size) = write_image_thumb(
        folder.path(),
        "photos/p.jpg",
        |_| (40, 30),
        &ThumbnailOptions::default(),
    );
    assert_eq!(size, (40, 30));
    let bytes = std::fs::read(folder.path().join(path)).unwrap();
    assert_eq!(
        hex::encode(Sha256::digest(bytes)),
        "5b3ee84dc8e3fee426431a1b46e3d7671df2c88ae71d14e35b7050c3727eeae0"
    );
}

// Section B.2.3
pub fn parse_sos<R: Read>(reader: &mut R, frame: &FrameInfo) -> Result<ScanInfo> {
    let length = read_length(reader, SOS)?;
    if 0 == length {
        return Err(Error::Format("zero length in SOS".to_owned()));
    }

    let component_count = read_u8(reader)?;

    if component_count == 0 || component_count > 4 {
        return Err(Error::Format(format!(
            "invalid component count {} in scan header",
            component_count
        )));
    }

    if length != 4 + 2 * component_count as usize {
        return Err(Error::Format("invalid length in SOS".to_owned()));
    }

    let mut component_indices = Vec::with_capacity(component_count as usize);
    let mut dc_table_indices = Vec::with_capacity(component_count as usize);
    let mut ac_table_indices = Vec::with_capacity(component_count as usize);

    for _ in 0..component_count {
        let identifier = read_u8(reader)?;

        let component_index = match frame
            .components
            .iter()
            .position(|c| c.identifier == identifier)
        {
            Some(value) => value,
            None => {
                return Err(Error::Format(format!(
                    "scan component identifier {} does not match any of the component identifiers defined in the frame",
                    identifier
                )));
            }
        };

        // Each of the scan's components must be unique.
        if component_indices.contains(&component_index) {
            return Err(Error::Format(format!(
                "duplicate scan component identifier {}",
                identifier
            )));
        }

        // "... the ordering in the scan header shall follow the ordering in the frame header."
        if component_index < *component_indices.iter().max().unwrap_or(&0) {
            return Err(Error::Format(
                "the scan component order does not follow the order in the frame header".to_owned(),
            ));
        }

        let byte = read_u8(reader)?;
        let dc_table_index = byte >> 4;
        let ac_table_index = byte & 0x0f;

        if dc_table_index > 3 || (frame.is_baseline && dc_table_index > 1) {
            return Err(Error::Format(format!(
                "invalid dc table index {}",
                dc_table_index
            )));
        }
        if ac_table_index > 3 || (frame.is_baseline && ac_table_index > 1) {
            return Err(Error::Format(format!(
                "invalid ac table index {}",
                ac_table_index
            )));
        }

        component_indices.push(component_index);
        dc_table_indices.push(dc_table_index as usize);
        ac_table_indices.push(ac_table_index as usize);
    }

    let blocks_per_mcu = component_indices
        .iter()
        .map(|&i| {
            frame.components[i].horizontal_sampling_factor as u32
                * frame.components[i].vertical_sampling_factor as u32
        })
        .sum::<u32>();

    if component_count > 1 && blocks_per_mcu > 10 {
        return Err(Error::Format(
            "scan with more than one component and more than 10 blocks per MCU".to_owned(),
        ));
    }

    // Also utilized as 'Predictor' in lossless coding, as MEAN in JPEG-LS etc.
    let spectral_selection_start = read_u8(reader)?;
    // Also utilized as ILV parameter in JPEG-LS.
    let mut spectral_selection_end = read_u8(reader)?;

    let byte = read_u8(reader)?;
    let successive_approximation_high = byte >> 4;
    let successive_approximation_low = byte & 0x0f;

    // The Differential Pulse-Mode prediction used (similar to png). Only utilized in Lossless
    // coding. Don't confuse with the JPEG-LS parameter coded using the same scan info portion.
    let predictor_selection;
    let point_transform = successive_approximation_low;

    if point_transform >= frame.precision {
        return Err(Error::Format(
            "invalid point transform, must be less than the frame precision".to_owned(),
        ));
    }

    if frame.coding_process == CodingProcess::DctProgressive {
        predictor_selection = Predictor::NoPrediction;
        if spectral_selection_end > 63
            || spectral_selection_start > spectral_selection_end
            || (spectral_selection_start == 0 && spectral_selection_end != 0)
        {
            return Err(Error::Format(format!(
                "invalid spectral selection parameters: ss={}, se={}",
                spectral_selection_start, spectral_selection_end
            )));
        }
        if spectral_selection_start != 0 && component_count != 1 {
            return Err(Error::Format(
                "spectral selection scan with AC coefficients can't have more than one component"
                    .to_owned(),
            ));
        }

        if successive_approximation_high > 13 || successive_approximation_low > 13 {
            return Err(Error::Format(format!(
                "invalid successive approximation parameters: ah={}, al={}",
                successive_approximation_high, successive_approximation_low
            )));
        }

        // Section G.1.1.1.2
        // "Each scan which follows the first scan for a given band progressively improves
        //     the precision of the coefficients by one bit, until full precision is reached."
        if successive_approximation_high != 0
            && successive_approximation_high != successive_approximation_low + 1
        {
            return Err(Error::Format(
                "successive approximation scan with more than one bit of improvement".to_owned(),
            ));
        }
    } else if frame.coding_process == CodingProcess::Lossless {
        if spectral_selection_end != 0 {
            return Err(Error::Format(
                "spectral selection end shall be zero in lossless scan".to_owned(),
            ));
        }
        if successive_approximation_high != 0 {
            return Err(Error::Format(
                "successive approximation high shall be zero in lossless scan".to_owned(),
            ));
        }
        predictor_selection = match spectral_selection_start {
            0 => Predictor::NoPrediction,
            1 => Predictor::Ra,
            2 => Predictor::Rb,
            3 => Predictor::Rc,
            4 => Predictor::RaRbRc1,
            5 => Predictor::RaRbRc2,
            6 => Predictor::RaRbRc3,
            7 => Predictor::RaRb,
            _ => {
                return Err(Error::Format(format!(
                    "invalid predictor selection value: {}",
                    spectral_selection_start
                )));
            }
        };
    } else {
        predictor_selection = Predictor::NoPrediction;
        if spectral_selection_end == 0 {
            spectral_selection_end = 63;
        }
        if spectral_selection_start != 0 || spectral_selection_end != 63 {
            return Err(Error::Format(
                "spectral selection is not allowed in non-progressive scan".to_owned(),
            ));
        }
        if successive_approximation_high != 0 || successive_approximation_low != 0 {
            return Err(Error::Format(
                "successive approximation is not allowed in non-progressive scan".to_owned(),
            ));
        }
    }

    Ok(ScanInfo {
        component_indices,
        dc_table_indices,
        ac_table_indices,
        spectral_selection: Range {
            start: spectral_selection_start,
            end: spectral_selection_end + 1,
        },
        predictor_selection,
        successive_approximation_high,
        successive_approximation_low,
        point_transform,
    })
}

mod tables;
pub use tables::{parse_app, parse_com, parse_dht, parse_dqt, parse_dri};
