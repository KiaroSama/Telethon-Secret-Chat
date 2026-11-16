use super::error::{Error, Result, UnsupportedFeature};
use super::huffman::{HuffmanDecoder, HuffmanTable, fill_default_mjpeg_tables};
use super::marker::Marker;
use super::parser::{
    AdobeColorTransform, AppData, CodingProcess, Component, Dimensions, EntropyCoding, FrameInfo,
    IccChunk, ScanInfo, parse_app, parse_com, parse_dht, parse_dqt, parse_dri, parse_sof,
    parse_sos,
};
use super::read_u8;
use super::upsampler::Upsampler;
use super::worker::{RowData, Worker, WorkerScope, compute_image_portable};
use core::cmp;
use core::mem;
use core::ops::Range;
use std::borrow::ToOwned;
use std::io::Read;
use std::sync::Arc;
use std::vec::Vec;
use std::{format, vec};

pub const MAX_COMPONENTS: usize = 4;

mod lossless;
use self::lossless::compute_image_lossless;

#[rustfmt::skip]
static UNZIGZAG: [u8; 64] = [
     0,  1,  8, 16,  9,  2,  3, 10,
    17, 24, 32, 25, 18, 11,  4,  5,
    12, 19, 26, 33, 40, 48, 41, 34,
    27, 20, 13,  6,  7, 14, 21, 28,
    35, 42, 49, 56, 57, 50, 43, 36,
    29, 22, 15, 23, 30, 37, 44, 51,
    58, 59, 52, 45, 38, 31, 39, 46,
    53, 60, 61, 54, 47, 55, 62, 63,
];

/// An enumeration over combinations of color spaces and bit depths a pixel can have.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PixelFormat {
    /// Luminance (grayscale), 8 bits
    L8,
    /// Luminance (grayscale), 16 bits
    L16,
    /// RGB, 8 bits per channel
    RGB24,
    /// CMYK, 8 bits per channel
    CMYK32,
}

impl PixelFormat {
    /// Determine the size in bytes of each pixel in this format
    pub fn pixel_bytes(&self) -> usize {
        match self {
            PixelFormat::L8 => 1,
            PixelFormat::L16 => 2,
            PixelFormat::RGB24 => 3,
            PixelFormat::CMYK32 => 4,
        }
    }
}

/// Represents metadata of an image.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageInfo {
    /// The width of the image, in pixels.
    pub width: u16,
    /// The height of the image, in pixels.
    pub height: u16,
    /// The pixel format of the image.
    pub pixel_format: PixelFormat,
    /// The coding process of the image.
    pub coding_process: CodingProcess,
}

/// Describes the colour transform to apply before binary data is returned
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[allow(clippy::upper_case_acronyms)]
pub enum ColorTransform {
    /// No transform should be applied and the data is returned as-is.
    None,
    /// Unknown colour transformation
    Unknown,
    /// Grayscale transform should be applied (expects 1 channel)
    Grayscale,
    /// RGB transform should be applied.
    RGB,
    /// YCbCr transform should be applied.
    YCbCr,
    /// CMYK transform should be applied.
    CMYK,
    /// YCCK transform should be applied.
    YCCK,
    /// big gamut Y/Cb/Cr, bg-sYCC
    JcsBgYcc,
    /// big gamut red/green/blue, bg-sRGB
    JcsBgRgb,
}

/// JPEG decoder
pub struct Decoder<R> {
    reader: R,

    frame: Option<FrameInfo>,
    dc_huffman_tables: Vec<Option<HuffmanTable>>,
    ac_huffman_tables: Vec<Option<HuffmanTable>>,
    quantization_tables: [Option<Arc<[u16; 64]>>; 4],

    restart_interval: u16,

    adobe_color_transform: Option<AdobeColorTransform>,
    color_transform: Option<ColorTransform>,

    is_jfif: bool,
    is_mjpeg: bool,

    icc_markers: Vec<IccChunk>,

    exif_data: Option<Vec<u8>>,
    xmp_data: Option<Vec<u8>>,
    psir_data: Option<Vec<u8>>,

    // Used for progressive JPEGs.
    coefficients: Vec<Vec<i16>>,
    // Bitmask of which coefficients has been completely decoded.
    coefficients_finished: [u64; MAX_COMPONENTS],

    // Maximum allowed size of decoded image buffer
    decoding_buffer_size_limit: usize,
}

impl<R: Read> Decoder<R> {
    /// Creates a new `Decoder` using the reader `reader`.
    pub fn new(reader: R) -> Decoder<R> {
        Decoder {
            reader,
            frame: None,
            dc_huffman_tables: vec![None, None, None, None],
            ac_huffman_tables: vec![None, None, None, None],
            quantization_tables: [None, None, None, None],
            restart_interval: 0,
            adobe_color_transform: None,
            color_transform: None,
            is_jfif: false,
            is_mjpeg: false,
            icc_markers: Vec::new(),
            exif_data: None,
            xmp_data: None,
            psir_data: None,
            coefficients: Vec::new(),
            coefficients_finished: [0; MAX_COMPONENTS],
            decoding_buffer_size_limit: usize::MAX,
        }
    }

    /// Colour transform to use when decoding the image. App segments relating to colour transforms
    /// will be ignored.
    pub fn set_color_transform(&mut self, transform: ColorTransform) {
        self.color_transform = Some(transform);
    }

    /// Set maximum buffer size allowed for decoded images
    pub fn set_max_decoding_buffer_size(&mut self, max: usize) {
        self.decoding_buffer_size_limit = max;
    }

    /// Returns metadata about the image.
    ///
    /// The returned value will be `None` until a call to either `read_info` or `decode` has
    /// returned `Ok`.
    pub fn info(&self) -> Option<ImageInfo> {
        match self.frame {
            Some(ref frame) => {
                let pixel_format = match frame.components.len() {
                    1 => match frame.precision {
                        2..=8 => PixelFormat::L8,
                        9..=16 => PixelFormat::L16,
                        _ => panic!(),
                    },
                    3 => PixelFormat::RGB24,
                    4 => PixelFormat::CMYK32,
                    _ => panic!(),
                };

                Some(ImageInfo {
                    width: frame.output_size.width,
                    height: frame.output_size.height,
                    pixel_format,
                    coding_process: frame.coding_process,
                })
            }
            None => None,
        }
    }

    /// Returns raw exif data, starting at the TIFF header, if the image contains any.
    ///
    /// The returned value will be `None` until a call to `decode` has returned `Ok`.
    pub fn exif_data(&self) -> Option<&[u8]> {
        self.exif_data.as_deref()
    }

    /// Returns the raw XMP packet if there is any.
    ///
    /// The returned value will be `None` until a call to `decode` has returned `Ok`.
    pub fn xmp_data(&self) -> Option<&[u8]> {
        self.xmp_data.as_deref()
    }

    /// Returns the embeded icc profile if the image contains one.
    pub fn icc_profile(&self) -> Option<Vec<u8>> {
        let mut marker_present: [Option<&IccChunk>; 256] = [None; 256];
        let num_markers = self.icc_markers.len();
        if num_markers == 0 || num_markers >= 255 {
            return None;
        }
        // check the validity of the markers
        for chunk in &self.icc_markers {
            if usize::from(chunk.num_markers) != num_markers {
                // all the lengths must match
                return None;
            }
            if chunk.seq_no == 0 {
                return None;
            }
            if marker_present[usize::from(chunk.seq_no)].is_some() {
                // duplicate seq_no
                return None;
            } else {
                marker_present[usize::from(chunk.seq_no)] = Some(chunk);
            }
        }

        // assemble them together by seq_no failing if any are missing
        let mut data = Vec::new();
        // seq_no's start at 1
        for &chunk in marker_present.get(1..=num_markers)? {
            data.extend_from_slice(&chunk?.data);
        }
        Some(data)
    }

    /// Tries to read metadata from the image without decoding it.
    ///
    /// If successful, the metadata can be obtained using the `info` method.
    pub fn read_info(&mut self) -> Result<()> {
        WorkerScope::with(|worker| self.decode_internal(true, worker)).map(|_| ())
    }

    /// Configure the decoder to scale the image during decoding.
    ///
    /// This efficiently scales the image by the smallest supported scale
    /// factor that produces an image larger than or equal to the requested
    /// size in at least one axis. The currently implemented scale factors
    /// are 1/8, 1/4, 1/2 and 1.
    ///
    /// To generate a thumbnail of an exact size, pass the desired size and
    /// then scale to the final size using a traditional resampling algorithm.
    pub fn scale(&mut self, requested_width: u16, requested_height: u16) -> Result<(u16, u16)> {
        self.read_info()?;
        let frame = self
            .frame
            .as_mut()
            .ok_or_else(|| Error::Format("missing frame".into()))?;
        let idct_size = super::idct::choose_idct_size(
            frame.image_size,
            Dimensions {
                width: requested_width,
                height: requested_height,
            },
        );
        frame.update_idct_size(idct_size)?;
        Ok((frame.output_size.width, frame.output_size.height))
    }

    /// Decodes the image and returns the decoded pixels if successful.
    pub fn decode(&mut self) -> Result<Vec<u8>> {
        WorkerScope::with(|worker| self.decode_internal(false, worker))
    }
}
mod control;
mod planes;
mod scans;
pub(crate) use planes::choose_color_convert_func;
