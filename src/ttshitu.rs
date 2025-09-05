use base64::{Engine as _, engine::general_purpose};
use image::DynamicImage;
use image::codecs::jpeg::JpegEncoder;
use std::io::Cursor;

const URL: &str = "http://api.ttshitu.com/base64";

#[derive(serde::Serialize, Debug)]
struct RecognizeData {
    username: String,
    password: String,
    typeid: usize,
    image: String,
}

#[derive(serde::Deserialize, Debug)]
struct ResData {
    data: ResDataResult,
}

#[derive(serde::Deserialize, Debug)]
struct ResDataResult {
    result: String,
}

pub async fn recognize(
    client: &cyper::Client,
    username: String,
    password: String,
    b64_image: String,
) -> anyhow::Result<String> {
    let body = serde_json::to_string(&RecognizeData {
        username,
        password,
        typeid: 3,
        image: b64_image,
    })?;

    let res = client
        .post(URL)?
        .header(http::header::CONTENT_TYPE, "application/json")?
        .body(body)
        .send()
        .await?;
    let res: ResData = serde_json::from_str(&res.text().await?)?;
    Ok(res.data.result)
}

/// 将 JPEG 二进制图片数据转换为 Base64 编码的 JPEG
pub fn jpeg_to_b64(raw: &[u8]) -> anyhow::Result<String> {
    let img: DynamicImage = image::load_from_memory(raw)?;

    // 转换为 RGB8 并编码为 JPEG
    let rgb = img.to_rgb8();
    let mut buffer = Vec::new();
    {
        let mut cursor = Cursor::new(&mut buffer);
        let mut encoder = JpegEncoder::new_with_quality(&mut cursor, 80);
        encoder.encode_image(&DynamicImage::ImageRgb8(rgb))?;
    }

    let b64 = general_purpose::STANDARD.encode(&buffer);
    Ok(b64)
}
