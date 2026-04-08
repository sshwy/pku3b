//! Low-level client that send http requests to the target server.

/// 教学网 API
pub mod blackboard;
/// 北京大学版权保护系统
#[cfg(feature = "thesislib")]
pub mod drm_lib;
/// IAAA 认证 API
pub mod iaaa;
/// 校内门户 API
pub mod portal;
/// 选课系统 API
pub mod syllabus;
/// 学位论文数据库
#[cfg(feature = "thesislib")]
pub mod thesis_lib;

use anyhow::Context as _;
use rand::Rng as _;
use scraper::Html;
use std::str::FromStr as _;

use crate::multipart;

/// Default User-Agent used by the crawler.
pub const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36";

/// 一个基础的爬虫 client，函数的返回内容均为原始的，未处理的信息.
#[derive(Clone)]
pub struct LowLevelClient {
    http_client: crate::http::Client,
}

impl LowLevelClient {
    pub fn new() -> Self {
        let mut default_headers = http::HeaderMap::new();
        default_headers.insert(http::header::USER_AGENT, USER_AGENT.parse().unwrap());
        let http_client = cyper::Client::builder()
            .cookie_store(true)
            .default_headers(default_headers)
            .build();

        Self {
            http_client: crate::http::Client::from_cyper(http_client),
        }
    }

    pub async fn load_set_cookies<P: AsRef<std::path::Path>>(&self, path: P) -> anyhow::Result<()> {
        self.http_client.load_set_cookies(path).await
    }

    #[cfg(feature = "thesislib")]
    fn encrypt_password(pubkey: &str, password: &str) -> anyhow::Result<String> {
        use base64::{Engine as _, engine::general_purpose};
        use pkcs8::DecodePublicKey;
        use rsa::{Pkcs1v15Encrypt, RsaPublicKey, rand_core::OsRng};

        // JSEncrypt: setPublicKey(SPKI PEM) then encrypt(plaintext) with RSAES-PKCS1-v1_5.
        let key = RsaPublicKey::from_public_key_pem(pubkey)
            .map_err(|e| anyhow::anyhow!("invalid public key PEM: {e}"))?;

        // PKCS#1 v1.5 encryption is randomized (non-zero padding), so ciphertext varies per call.
        let mut rng = OsRng;
        let ciphertext = key
            .encrypt(&mut rng, Pkcs1v15Encrypt, password.as_bytes())
            .map_err(|e| anyhow::anyhow!("rsa encrypt failed: {e}"))?;

        Ok(general_purpose::STANDARD.encode(ciphertext))
    }

    /// 利用 [`convert_uri`] 将 uri 自动补全，然后发送请求.
    pub async fn get_by_uri(&self, uri: &str) -> anyhow::Result<cyper::Response> {
        let url = convert_uri(uri)?;
        log::trace!("GET {url}");
        let res = self
            .http_client
            .get(url)
            .context("create request failed")?
            .send()
            .await?;
        Ok(res)
    }

    /// 利用 [`convert_uri`] 将 uri 自动补全，然后发送请求, 返回页面 HTML
    #[allow(unused)]
    pub async fn page_by_uri(&self, uri: &str) -> anyhow::Result<Html> {
        let res = self.get_by_uri(uri).await?;

        anyhow::ensure!(res.status().is_success(), "status not success");

        let rbody = res.text().await?;
        let dom = scraper::Html::parse_document(&rbody);
        Ok(dom)
    }

    #[cfg(feature = "ttshitu")]
    pub async fn ttshitu_recognize(
        &self,
        username: String,
        password: String,
        image_b64: String,
    ) -> anyhow::Result<String> {
        crate::ttshitu::recognize(&self.http_client, username, password, image_b64).await
    }
}

/// 将 uri 转换为完整的 url。协议默认为 `https`，域名默认为 `course.pku.edu.cn`。
pub fn convert_uri(uri: &str) -> anyhow::Result<String> {
    let uri = http::Uri::from_str(uri).context("parse uri string")?;
    let http::uri::Parts {
        scheme,
        authority,
        path_and_query,
        ..
    } = uri.into_parts();

    let url = format!(
        "{}://{}{}",
        scheme.as_ref().map(|s| s.as_str()).unwrap_or("https"),
        authority
            .as_ref()
            .map(|a| a.as_str())
            .unwrap_or("course.pku.edu.cn"),
        path_and_query.as_ref().map(|p| p.as_str()).unwrap_or(""),
    );

    Ok(url)
}

/// Extracts the redirect URL from a response with a redirection status.
///
/// # Arguments
///
/// * `res` - A reference to the [`cyper::Response`] object from which to extract the "Location"
///   header if it is a redirection.
///
/// # Returns
///
/// Returns a string slice representing the URL found in the `Location` header if present.
///
/// # Errors
///
/// This function returns an error if:
/// - The response status is not a redirection.
/// - The "Location" header is missing.
/// - The value of the "Location" header cannot be converted to a valid string.
fn extract_redirect_url(res: &cyper::Response) -> anyhow::Result<&str> {
    anyhow::ensure!(
        res.status().is_redirection(),
        "expect redirection, but got status {}",
        res.status()
    );
    let Some(url) = res.headers().get("Location") else {
        anyhow::bail!("location header not found");
    };
    let url = url.to_str().context("location header not valid str")?;
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    #[test]
    fn test_convert_uri() {
        let uri = "/path/to/resource";
        let expected = "https://course.pku.edu.cn/path/to/resource";
        let result = convert_uri(uri).unwrap();
        assert_eq!(result, expected);

        let uri = "http://example.com/path/to/resource";
        let expected = "http://example.com/path/to/resource";
        let result = convert_uri(uri).unwrap();
        assert_eq!(result, expected);

        let uri = "https://example.com/path/to/resource";
        let expected = "https://example.com/path/to/resource";
        let result = convert_uri(uri).unwrap();
        assert_eq!(result, expected);
    }

    const HAR_PEM_PUBLIC_KEY: &str = r#"-----BEGIN PUBLIC KEY-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAqw9PsMk8v9ED/LiLT62I
DnelyIA/s8blyxqNmbgXT4xtq+Y64Bd+THYPZ4dUIRuFmMvPowQm9wL27W3PEtQy
C8VN+TzW/nPzc74fy9cRxgaSh1FXNQBqYZtltb6G5YvwBvZlYdKhE3Oo3noUD0FJ
JC11Nmcy2/x1V2pwXHRy2DHKaWB1EEtQ9dRxuMZolZIpEwWnT4CHfwEvth83kNRp
E8471KJEqyQqmqJt3JRerH4X4p41zQFIxCsrznAwku3b1qm0vgGLQ8t7XEiCjDX0
m5yIJEuW5t1YcteutuJX5+5oXxe2Fo04Wkn1pO6+QoJopqHcHJD5C+7GlnPOLB1c
DQIDAQAB
-----END PUBLIC KEY-----
"#;

    #[test]
    #[cfg(feature = "thesislib")]
    fn encrypt_password_outputs_256b_ciphertext_base64() {
        let enc = LowLevelClient::encrypt_password(HAR_PEM_PUBLIC_KEY, "123123123123").unwrap();

        let raw = base64::engine::general_purpose::STANDARD
            .decode(enc)
            .unwrap();
        assert_eq!(raw.len(), 256);
    }
}
