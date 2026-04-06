//! Low-level client that send http requests to the target server.

/// 教学网 API
pub mod blackboard;
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

pub const IAAA_IS_MOBILE_AUTHEN: &str = "https://iaaa.pku.edu.cn/iaaa/isMobileAuthen.do";
pub const IAAA_OAUTH_LOGIN: &str = "https://iaaa.pku.edu.cn/iaaa/oauthlogin.do";
pub const IAAA_PUBKEY: &str = "https://iaaa.pku.edu.cn/iaaa/getPublicKey.do";

/// OAuth login error codes:
///
/// - E05: OTP code incorrect
/// - E21: Too many attempts. Please sign in after a half hour.
///
#[derive(serde::Deserialize, Debug)]
pub struct OAuthLoginError {
    pub code: String,
    pub msg: String,
}

impl std::error::Error for OAuthLoginError {}

impl std::fmt::Display for OAuthLoginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "OAuth login error [{}]: msg={}", self.code, self.msg)
    }
}

#[derive(serde::Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
#[allow(unused)]
pub struct AuthenData {
    pub authen_mode: String,
    pub bz_auth_mode: String,
    pub is_bind: bool,
    pub is_mobile_authen: bool,
    pub is_unu_auth: bool,
    pub mobile_mask: String,
    pub success: bool,
}

impl AuthenData {
    pub fn is_no(&self) -> bool {
        self.authen_mode == "否"
    }
}

/// 一个基础的爬虫 client，函数的返回内容均为原始的，未处理的信息.
#[derive(Clone)]
pub struct LowLevelClient {
    http_client: cyper::Client,
}

impl LowLevelClient {
    pub fn new() -> Self {
        let mut default_headers = http::HeaderMap::new();
        default_headers.insert(http::header::USER_AGENT, USER_AGENT.parse().unwrap());
        let http_client = cyper::Client::builder()
            .cookie_store(true)
            .default_headers(default_headers)
            .build();

        Self::from_cyper_client(http_client)
    }
    pub fn from_cyper_client(client: cyper::Client) -> Self {
        Self {
            http_client: client,
        }
    }

    /// 向 [`IAAA_OAUTH_LOGIN`] 发送登录请求，并返回 token
    pub(crate) async fn oauth_login(
        &self,
        appid: &str,
        username: &str,
        password: &str,
        redir: &str,
    ) -> anyhow::Result<String> {
        let res = self
            .http_client
            .post(IAAA_OAUTH_LOGIN)?
            .form(&[
                ("appid", appid),
                ("userName", username),
                ("password", password),
                ("randCode", ""),
                ("smsCode", ""),
                ("otpCode", ""),
                ("redirUrl", redir),
            ])?
            .send()
            .await?;

        anyhow::ensure!(
            res.status().is_success(),
            "oauth login not success: {}",
            res.status()
        );

        let rbody = res.text().await?;

        #[derive(serde::Deserialize)]
<<<<<<< HEAD
        struct OAuthLoginData {
            token: Option<String>,
            errors: Option<OAuthLoginError>,
        }
        let data: OAuthLoginData = serde_json::from_str(&rbody)
            .context("fail to parse response")
            .inspect_err(|e| {
                log::debug!("{e}");
                log::debug!("response body: {rbody}")
            })?;
=======
        struct ResData {
            success: bool,
            token: String,
        }
        let data: ResData = serde_json::from_str(&rbody).context("fail to parse response")?;
        anyhow::ensure!(data.success, "oauth login not success");
>>>>>>> 8c3015a (feat: add IAAA SSO login with JSEncrypt-compatible RSA password encryption)

        if let Some(err) = data.errors {
            return Err(err.into());
        }

        data.token.context("token not found")
    }

    async fn iaaa_is_mobile_authen(
        &self,
        appid: &str,
        username: &str,
    ) -> anyhow::Result<AuthenData> {
        let mut rng = rand::rng();

        let _rand: f64 = rng.sample(rand::distr::Open01);
        let _rand = format!("{_rand:.20}");

        let res = self
            .http_client
            .get(IAAA_IS_MOBILE_AUTHEN)?
            .query(&[
                ("appId", appid),
                ("userName", username),
                ("_rand", _rand.as_str()),
            ])?
            .send()
            .await?;

        let rbody = res.text().await?;
        let data: AuthenData = serde_json::from_str(&rbody).context("fail to parse response")?;
        Ok(data)
    }

    #[cfg(feature = "thesislib")]
    async fn iaaa_public_key(&self) -> anyhow::Result<String> {
        let res = self.get_by_uri(IAAA_PUBKEY).await?;
        anyhow::ensure!(res.status().is_success(), "error status {}", res.status());

        #[derive(serde::Deserialize)]
        struct Data {
            success: bool,
            key: String,
        }

        let data: Data = serde_json::from_str(&res.text().await?)?;
        anyhow::ensure!(data.success, "get pubkey failed");

        Ok(data.key)
    }

    #[cfg(feature = "thesislib")]
    fn encrypt_password(&self, pubkey: &str, password: &str) -> anyhow::Result<String> {
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
        // Note: we don't need a real HTTP client; only the encryption helper is used.
        let client = LowLevelClient::new();
        let enc = client
            .encrypt_password(HAR_PEM_PUBLIC_KEY, "123123123123")
            .unwrap();

        let raw = base64::engine::general_purpose::STANDARD
            .decode(enc)
            .unwrap();
        assert_eq!(raw.len(), 256);
    }
}
