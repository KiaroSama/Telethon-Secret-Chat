//! Safe JPEG metadata and quantization/Huffman parsing; image-rs 0.3.2 adaptation.
use super::*;

// Section B.2.4.1
pub fn parse_dqt<R: Read>(reader: &mut R) -> Result<[Option<[u16; 64]>; 4]> {
    let mut length = read_length(reader, DQT)?;
    let mut tables = [None; 4];

    // Each DQT segment may contain multiple quantization tables.
    while length > 0 {
        let byte = read_u8(reader)?;
        let precision = (byte >> 4) as usize;
        let index = (byte & 0x0f) as usize;

        // The combination of 8-bit sample precision and 16-bit quantization tables is explicitly
        // disallowed by the JPEG spec:
        //     "An 8-bit DCT-based process shall not use a 16-bit precision quantization table."
        //     "Pq: Quantization table element precision – Specifies the precision of the Qk
        //      values. Value 0 indicates 8-bit Qk values; value 1 indicates 16-bit Qk values. Pq
        //      shall be zero for 8 bit sample precision P (see B.2.2)."
        // libjpeg allows this behavior though, and there are images in the wild using it. So to
        // match libjpeg's behavior we are deviating from the JPEG spec here.
        if precision > 1 {
            return Err(Error::Format(format!(
                "invalid precision {} in DQT",
                precision
            )));
        }
        if index > 3 {
            return Err(Error::Format(format!(
                "invalid destination identifier {} in DQT",
                index
            )));
        }
        if length < 65 + 64 * precision {
            return Err(Error::Format("invalid length in DQT".to_owned()));
        }

        let mut table = [0u16; 64];

        for item in table.iter_mut() {
            *item = match precision {
                0 => u16::from(read_u8(reader)?),
                1 => read_u16_from_be(reader)?,
                _ => unreachable!(),
            };
        }

        if table.contains(&0) {
            return Err(Error::Format(
                "quantization table contains element with a zero value".to_owned(),
            ));
        }

        tables[index] = Some(table);
        length -= 65 + 64 * precision;
    }

    Ok(tables)
}

// Section B.2.4.2
#[allow(clippy::type_complexity)]
pub fn parse_dht<R: Read>(
    reader: &mut R,
    is_baseline: Option<bool>,
) -> Result<(Vec<Option<HuffmanTable>>, Vec<Option<HuffmanTable>>)> {
    let mut length = read_length(reader, DHT)?;
    let mut dc_tables = vec![None, None, None, None];
    let mut ac_tables = vec![None, None, None, None];

    // Each DHT segment may contain multiple huffman tables.
    while length > 17 {
        let byte = read_u8(reader)?;
        let class = byte >> 4;
        let index = (byte & 0x0f) as usize;

        if class != 0 && class != 1 {
            return Err(Error::Format(format!("invalid class {} in DHT", class)));
        }
        if is_baseline == Some(true) && index > 1 {
            return Err(Error::Format(
                "a maximum of two huffman tables per class are allowed in baseline".to_owned(),
            ));
        }
        if index > 3 {
            return Err(Error::Format(format!(
                "invalid destination identifier {} in DHT",
                index
            )));
        }

        let mut counts = [0u8; 16];
        reader.read_exact(&mut counts)?;

        let size = counts.iter().map(|&val| val as usize).sum::<usize>();

        if size == 0 {
            return Err(Error::Format(
                "encountered table with zero length in DHT".to_owned(),
            ));
        } else if size > 256 {
            return Err(Error::Format(
                "encountered table with excessive length in DHT".to_owned(),
            ));
        } else if size > length - 17 {
            return Err(Error::Format("invalid length in DHT".to_owned()));
        }

        let mut values = vec![0u8; size];
        reader.read_exact(&mut values)?;

        match class {
            0 => {
                dc_tables[index] = Some(HuffmanTable::new(&counts, &values, HuffmanTableClass::DC)?)
            }
            1 => {
                ac_tables[index] = Some(HuffmanTable::new(&counts, &values, HuffmanTableClass::AC)?)
            }
            _ => unreachable!(),
        }

        length -= 17 + size;
    }

    if length != 0 {
        return Err(Error::Format("invalid length in DHT".to_owned()));
    }

    Ok((dc_tables, ac_tables))
}

// Section B.2.4.4
pub fn parse_dri<R: Read>(reader: &mut R) -> Result<u16> {
    let length = read_length(reader, DRI)?;

    if length != 2 {
        return Err(Error::Format("DRI with invalid length".to_owned()));
    }

    Ok(read_u16_from_be(reader)?)
}

// Section B.2.4.5
pub fn parse_com<R: Read>(reader: &mut R) -> Result<Vec<u8>> {
    let length = read_length(reader, COM)?;
    let mut buffer = vec![0u8; length];

    reader.read_exact(&mut buffer)?;

    Ok(buffer)
}

// Section B.2.4.6
pub fn parse_app<R: Read>(reader: &mut R, marker: Marker) -> Result<Option<AppData>> {
    let length = read_length(reader, marker)?;
    let mut bytes_read = 0;
    let mut result = None;

    match marker {
        APP(0) => {
            if length >= 5 {
                let mut buffer = [0u8; 5];
                reader.read_exact(&mut buffer)?;
                bytes_read = buffer.len();

                // http://www.w3.org/Graphics/JPEG/jfif3.pdf
                if buffer[0..5] == *b"JFIF\0" {
                    result = Some(AppData::Jfif);
                // https://sno.phy.queensu.ca/~phil/exiftool/TagNames/JPEG.html#AVI1
                } else if buffer[0..5] == *b"AVI1\0" {
                    result = Some(AppData::Avi1);
                }
            }
        }
        APP(1) => {
            let mut buffer = vec![0u8; length];
            reader.read_exact(&mut buffer)?;
            bytes_read = buffer.len();

            // https://web.archive.org/web/20190624045241if_/http://www.cipa.jp:80/std/documents/e/DC-008-Translation-2019-E.pdf
            // 4.5.4 Basic Structure of JPEG Compressed Data
            if length >= 6 && buffer[0..6] == *b"Exif\x00\x00" {
                result = Some(AppData::Exif(buffer[6..].to_vec()));
            }
            // XMP packet
            // https://github.com/adobe/XMP-Toolkit-SDK/blob/main/docs/XMPSpecificationPart3.pdf
            else if length >= 29 && buffer[0..29] == *b"http://ns.adobe.com/xap/1.0/\0" {
                result = Some(AppData::Xmp(buffer[29..].to_vec()));
            }
        }
        APP(2) => {
            if length > 14 {
                let mut buffer = [0u8; 14];
                reader.read_exact(&mut buffer)?;
                bytes_read = buffer.len();

                // http://www.color.org/ICC_Minor_Revision_for_Web.pdf
                // B.4 Embedding ICC profiles in JFIF files
                if buffer[0..12] == *b"ICC_PROFILE\0" {
                    let mut data = vec![0; length - bytes_read];
                    reader.read_exact(&mut data)?;
                    bytes_read += data.len();
                    result = Some(AppData::Icc(IccChunk {
                        seq_no: buffer[12],
                        num_markers: buffer[13],
                        data,
                    }));
                }
            }
        }
        APP(13) => {
            if length >= 14 {
                let mut buffer = [0u8; 14];
                reader.read_exact(&mut buffer)?;
                bytes_read = buffer.len();

                // PSIR (Photoshop)
                // https://github.com/adobe/XMP-Toolkit-SDK/blob/main/docs/XMPSpecificationPart3.pdf
                if buffer[0..14] == *b"Photoshop 3.0\0" {
                    let mut data = vec![0; length - bytes_read];
                    reader.read_exact(&mut data)?;
                    bytes_read += data.len();
                    result = Some(AppData::Psir(data));
                }
            }
        }
        APP(14) if length >= 12 => {
            let mut buffer = [0u8; 12];
            reader.read_exact(&mut buffer)?;
            bytes_read = buffer.len();

            // http://www.sno.phy.queensu.ca/~phil/exiftool/TagNames/JPEG.html#Adobe
            if buffer[0..6] == *b"Adobe\0" {
                let color_transform = match buffer[11] {
                    0 => AdobeColorTransform::Unknown,
                    1 => AdobeColorTransform::YCbCr,
                    2 => AdobeColorTransform::YCCK,
                    _ => {
                        return Err(Error::Format(
                            "invalid color transform in adobe app segment".to_owned(),
                        ));
                    }
                };

                result = Some(AppData::Adobe(color_transform));
            }
        }
        _ => {}
    }

    skip_bytes(reader, length - bytes_read)?;
    Ok(result)
}
