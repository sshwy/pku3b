//! Low-level client that send http requests to the target server.

/// 教学网 API
pub mod blackboard;
/// 选课系统 API
pub mod syllabus;

use anyhow::Context as _;
use rand::Rng as _;
use scraper::Html;
use std::str::FromStr as _;

use crate::multipart;

/// Default User-Agent used by the crawler.
pub const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36";

pub const OAUTH_LOGIN: &str = "https://iaaa.pku.edu.cn/iaaa/oauthlogin.do";

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

    /// 向 [`OAUTH_LOGIN`] 发送登录请求，并返回 token
    async fn oauth_login(
        &self,
        appid: &str,
        username: &str,
        password: &str,
        redir: &str,
    ) -> anyhow::Result<String> {
        let res = self
            .http_client
            .post(OAUTH_LOGIN)?
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
        struct ResData {
            token: String,
        }
        let data: ResData = serde_json::from_str(&rbody).context("fail to parse response")?;

        Ok(data.token)
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
}
