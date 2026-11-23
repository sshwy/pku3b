use base64::{Engine as _, engine::general_purpose};
use image::DynamicImage;
use image::codecs::jpeg::JpegEncoder;
use std::io::Cursor;

const URL: &str = "https://api.ttshitu.com/base64";

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
    client: &crate::http::Client,
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
    parse_response(&res.text().await?)
}

/// 解析识别结果。出错时（账号密码错误、余额不足等）`data` 是空串，此时报告 `message`。
fn parse_response(body: &str) -> anyhow::Result<String> {
    #[derive(serde::Deserialize)]
    struct Status {
        success: bool,
        #[serde(default)]
        message: String,
    }
    let status: Status = serde_json::from_str(body)?;
    anyhow::ensure!(status.success, "TT 识图返回错误: {}", status.message);

    let res: ResData = serde_json::from_str(body)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_response_reports_api_error() {
        // 账号密码错误、余额不足等情况下 data 是空串
        let body =
            r#"{"success":false,"code":"-1","message":"用户名或密码错误,检查后重试","data":""}"#;
        let err = parse_response(body).unwrap_err();
        assert!(err.to_string().contains("用户名或密码错误"), "{err}");
    }

    #[test]
    fn parse_response_returns_result() {
        let body =
            r#"{"success":true,"code":"0","message":"success","data":{"result":"ab12","id":"x"}}"#;
        assert_eq!(parse_response(body).unwrap(), "ab12");
    }
}
