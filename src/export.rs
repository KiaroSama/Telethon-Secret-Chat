use serde_json::Value;

use crate::errors::SecretChatError;

pub fn escape_html_text(text: &str) -> Result<String, SecretChatError> {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    Ok(out)
}

pub fn safe_http_href(url: &str) -> Result<Option<String>, SecretChatError> {
    crate::ported::tdexport_html_safe::safe_http_href(url.to_owned()).map_err(|err| {
        SecretChatError::ParameterRejected {
            chat_id: None,
            reason: err,
        }
    })
}

pub fn parse_strict_qurl(text: &str) -> Result<String, SecretChatError> {
    let parsed =
        crate::ported::tdexport_html_qurl::parse_strict(text.to_owned()).map_err(|err| {
            SecretChatError::ParameterRejected {
                chat_id: None,
                reason: err,
            }
        })?;
    crate::ported::tdexport_html_qurl::to_encoded(parsed).map_err(|err| {
        SecretChatError::ParameterRejected {
            chat_id: None,
            reason: err,
        }
    })
}

pub fn mime_first_glob(mime: &str) -> Result<String, SecretChatError> {
    crate::ported::tdexport_mime_globs::mime_first_glob(mime.to_owned()).map_err(|err| {
        SecretChatError::ParameterRejected {
            chat_id: None,
            reason: err,
        }
    })
}

pub fn format_amount_and_currency(
    amount: i64,
    currency: &str,
    force_strip_dot_zero: bool,
) -> Result<String, SecretChatError> {
    crate::ported::tdexport_model_format::fill_amount_and_currency(
        amount,
        currency.to_owned(),
        force_strip_dot_zero,
    )
    .map_err(|err| SecretChatError::ParameterRejected {
        chat_id: None,
        reason: err,
    })
}

pub fn serialize_json_string(value: &str) -> Result<String, SecretChatError> {
    let bytes = crate::ported::tdexport_json_serialize::serialize_string(value.to_owned())
        .map_err(|err| SecretChatError::ParameterRejected {
            chat_id: None,
            reason: err,
        })?;
    String::from_utf8(bytes).map_err(|_| SecretChatError::ParameterRejected {
        chat_id: None,
        reason: "serialized JSON string is not valid UTF-8".into(),
    })
}

pub fn convert_icc_to_srgb(
    pixels: &[u8],
    shape: &[i64],
    icc_profile: &[u8],
) -> Result<(Vec<u8>, Vec<u8>), SecretChatError> {
    let val = crate::ported::tdexport_qt_icc::convert_to_srgb(
        pixels.to_vec(),
        shape.to_vec(),
        icc_profile.to_vec(),
    )
    .map_err(|err| SecretChatError::ParameterRejected {
        chat_id: None,
        reason: err,
    })?;
    let pair = val["$tuple"]
        .as_array()
        .ok_or_else(|| SecretChatError::ParameterRejected {
            chat_id: None,
            reason: "malformed ICC conversion result".into(),
        })?;
    let out_pixels =
        extract_tagged_bytes(&pair[0]).ok_or_else(|| SecretChatError::ParameterRejected {
            chat_id: None,
            reason: "malformed ICC output pixels".into(),
        })?;
    let out_profile =
        extract_tagged_bytes(&pair[1]).ok_or_else(|| SecretChatError::ParameterRejected {
            chat_id: None,
            reason: "malformed ICC output profile".into(),
        })?;
    Ok((out_pixels, out_profile))
}

pub fn jpeg_source_metadata(data: &[u8]) -> Result<Value, SecretChatError> {
    crate::ported::tdexport_qt_jpeg::source_metadata(data.to_vec()).map_err(|err| {
        SecretChatError::ParameterRejected {
            chat_id: None,
            reason: err,
        }
    })
}

fn extract_tagged_bytes(value: &Value) -> Option<Vec<u8>> {
    let arr = value.get("$bytes")?.as_array()?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let byte = item.as_u64()?;
        out.push(u8::try_from(byte).ok()?);
    }
    Some(out)
}
