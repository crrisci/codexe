//! Inline media uses Codex's retained bytes; no paths, downloads or extra media store.
use crate::protocol::{Describe, invalid};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};
use std::io;

const MAX_IMAGE_BYTES: usize = 48 * 1024 * 1024;

pub(crate) fn content(value: &Value) -> io::Result<Value> {
    if let Some(text) = value.as_str() {
        return Ok(json!({"text": text}));
    }
    let parts = value
        .as_array()
        .ok_or_else(|| invalid("expected message content"))?;
    let mut text = String::new();
    let mut image = None;
    for part in parts {
        match part["type"].as_str() {
            Some("input_text" | "output_text") => text.push_str(
                part["text"]
                    .as_str()
                    .ok_or_else(|| invalid("missing text"))?,
            ),
            Some("input_image") => {
                if image.is_some() {
                    return Err(invalid(
                        "Xenolith supports one image per message or tool result; send images in separate turns",
                    ));
                }
                let url = part["image_url"].as_str().ok_or_else(|| {
                    invalid("Xenolith requires inline image bytes, not a file ID")
                })?;
                let (mime, data) = url.strip_prefix("data:").and_then(|url| url.split_once(";base64,")).ok_or_else(|| invalid("Xenolith requires a PNG/JPEG Base64 data URL; remote image URLs are unsupported"))?;
                if data.is_empty() || data.len() > MAX_IMAGE_BYTES.div_ceil(3) * 4 {
                    return Err(invalid(
                        "image exceeds the supported byte limit or is empty",
                    ));
                }
                let bytes = STANDARD
                    .decode(data)
                    .map_err(|_| invalid("invalid image Base64"))?;
                if bytes.len() > MAX_IMAGE_BYTES || STANDARD.encode(&bytes) != data {
                    return Err(invalid(
                        "image must use canonical Base64 within the supported byte limit",
                    ));
                }
                let format = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
                    "image/png"
                } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
                    "image/jpeg"
                } else {
                    return Err(invalid("Xenolith accepts PNG and JPEG images only"));
                };
                if mime != format {
                    return Err(invalid("image MIME type does not match its PNG/JPEG bytes"));
                }
                image = Some(json!({"data": data}));
            }
            _ => return Err(invalid("unsupported Xenolith message content")),
        }
    }
    let mut message = json!({"text": text});
    if let Some(image) = image {
        message["image"] = image;
    }
    Ok(message)
}

pub(crate) fn configure(
    messages: &mut [Value],
    info: &Describe,
    requested: Option<u32>,
) -> io::Result<Option<u32>> {
    let mut budget = None;
    let mut total_bytes = 0u64;
    for message in messages {
        let Some(image) = message.get_mut("image") else {
            continue;
        };
        let vision = info.vision.as_ref().ok_or_else(|| {
            invalid("the Xenolith service does not support images; start it with --mmproj")
        })?;
        let selected = requested.unwrap_or(vision.default_budget);
        if !vision.budgets.contains(&selected) {
            return Err(invalid("image budget is not advertised by Xenolith"));
        }
        let data = image["data"].as_str().expect("validated inline image");
        let bytes =
            (data.len() / 4 * 3 - data.bytes().rev().take_while(|b| *b == b'=').count()) as u64;
        let format = if data.starts_with("iVBOR") {
            "png"
        } else {
            "jpeg"
        };
        if !vision.formats.iter().any(|value| value == format) || bytes > vision.max_bytes {
            return Err(invalid(
                "image format or size exceeds the Xenolith service capabilities",
            ));
        }
        total_bytes = total_bytes
            .checked_add(bytes)
            .ok_or_else(|| invalid("image size overflow"))?;
        if total_bytes > vision.max_context_media_bytes {
            return Err(invalid(
                "images exceed the Xenolith context media byte limit",
            ));
        }
        image["budget"] = json!(selected);
        budget = Some(selected);
    }
    Ok(budget)
}
