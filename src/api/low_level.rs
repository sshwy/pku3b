//! Low-level client that send http requests to the target server.

use anyhow::Context as _;
use rand::Rng as _;
use scraper::Html;
use std::str::FromStr as _;

use crate::multipart;

const OAUTH_LOGIN: &str = "https://iaaa.pku.edu.cn/iaaa/oauthlogin.do";
const REDIR_URL: &str =
    "http://course.pku.edu.cn/webapps/bb-sso-BBLEARN/execute/authValidate/campusLogin";
const SSO_LOGIN: &str =
    "https://course.pku.edu.cn/webapps/bb-sso-BBLEARN/execute/authValidate/campusLogin";
const BLACKBOARD_HOME_PAGE: &str =
    "https://course.pku.edu.cn/webapps/portal/execute/tabs/tabAction";
const COURSE_INFO_PAGE: &str = "https://course.pku.edu.cn/webapps/blackboard/execute/announcement";
const UPLOAD_ASSIGNMENT: &str = "https://course.pku.edu.cn/webapps/assignment/uploadAssignment";
const VIDEO_SUB_INFO: &str =
    "https://yjapise.pku.edu.cn/courseapi/v2/schedule/get-sub-info-by-auth-data";

/// 一个基础的爬虫 client，函数的返回内容均为原始的，未处理的信息.
#[derive(Clone)]
pub struct LowLevelClient {
    http_client: cyper::Client,
}

impl LowLevelClient {
    pub fn from_cyper_client(client: cyper::Client) -> Self {
        Self {
            http_client: client,
        }
    }

    /// 向 [`OAUTH_LOGIN`] 发送登录请求，并返回 JSON (形如 { token: "..." })
    pub async fn oauth_login(
        &self,
        username: &str,
        password: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let res = self
            .http_client
            .post(OAUTH_LOGIN)?
            .form(&[
                ("appid", "blackboard"),
                ("userName", username),
                ("password", password),
                ("randCode", ""),
                ("smsCode", ""),
                ("otpCode", ""),
                ("redirUrl", REDIR_URL),
            ])?
            .send()
            .await?;

        anyhow::ensure!(
            res.status().is_success(),
            "oauth login not success: {}",
            res.status()
        );

        let rbody = res.text().await?;
        let value = serde_json::Value::from_str(&rbody).context("fail to parse response json")?;
        Ok(value)
    }

    /// 使用 OAuth login 返回的 token 登录教学网。登录状态会记录在 client cookie 中，无需返回值.
    pub async fn blackboard_sso_login(&self, token: &str) -> anyhow::Result<()> {
        let mut rng = rand::rng();

        let _rand: f64 = rng.sample(rand::distr::Open01);
        let _rand = format!("{_rand:.20}");

        let res = self
            .http_client
            .get(SSO_LOGIN)?
            .query(&[("_rand", _rand.as_str()), ("token", token)])?
            .send()
            .await?;

        anyhow::ensure!(res.status().is_success(), "status not success");

        Ok(())
    }

    /// 获取教学网主页内容 ([`BLACKBOARD_HOME_PAGE`]), 返回 HTML 文档
    pub async fn blackboard_homepage(&self) -> anyhow::Result<Html> {
        let res = self
            .http_client
            .get(BLACKBOARD_HOME_PAGE)?
            .query(&[("tab_tab_group_id", "_1_1")])?
            .send()
            .await?;

        anyhow::ensure!(res.status().is_success(), "status not success");

        let rbody = res.text().await?;
        let dom = scraper::Html::parse_document(&rbody);
        Ok(dom)
    }

    /// 根据课程的 key 获取课程主页内容 ([`COURSE_INFO_PAGE`])
    pub async fn blackboard_coursepage(&self, key: &str) -> anyhow::Result<Html> {
        let res = self
            .http_client
            .get(COURSE_INFO_PAGE)?
            .query(&[
                ("method", "search"),
                ("context", "course_entry"),
                ("course_id", key),
                ("handle", "announcements_entry"),
                ("mode", "view"),
            ])?
            .send()
            .await?;

        anyhow::ensure!(res.status().is_success(), "status not success");

        let rbody = res.text().await?;
        let dom = scraper::Html::parse_document(&rbody);
        Ok(dom)
    }

    /// 根据 content_id 和 course_id 获取作业上传页面的信息.
    pub async fn blackboard_course_assignment_uploadpage(
        &self,
        course_id: &str,
        content_id: &str,
    ) -> anyhow::Result<Html> {
        let res = self
            .http_client
            .get(UPLOAD_ASSIGNMENT)?
            .query(&[
                ("action", "newAttempt"),
                ("content_id", content_id),
                ("course_id", course_id),
            ])?
            .send()
            .await?;

        anyhow::ensure!(res.status().is_success(), "status not success");

        let rbody = res.text().await?;
        let dom = scraper::Html::parse_document(&rbody);
        Ok(dom)
    }

    /// 根据 content_id 和 course_id 获取作业的历史提交页面.
    pub async fn blackboard_course_assignment_viewpage(
        &self,
        course_id: &str,
        content_id: &str,
    ) -> anyhow::Result<Html> {
        let res = self
            .http_client
            .get(UPLOAD_ASSIGNMENT)?
            .query(&[
                ("mode", "view"),
                ("content_id", content_id),
                ("course_id", course_id),
            ])?
            .send()
            .await?;

        anyhow::ensure!(res.status().is_success(), "status not success");

        let rbody = res.text().await?;
        let dom = scraper::Html::parse_document(&rbody);
        Ok(dom)
    }

    /// 向 [`UPLOAD_ASSIGNMENT`] 发送提交作业的请求
    pub async fn blackboard_course_assignment_uploaddata(
        &self,
        body: multipart::MultipartBuilder<'_>,
    ) -> anyhow::Result<cyper::Response> {
        let boundary = body.boundary().to_owned();
        let body = body.build().context("build multipart form body")?;

        log::debug!("body built: {}", body.len());

        let res = self
            .http_client
            .post(UPLOAD_ASSIGNMENT)?
            .header("origin", "https://course.pku.edu.cn")?
            .header("accept", "*/*")?
            .header(
                "content-type",
                format!("multipart/form-data; boundary={}", boundary),
            )?
            .query(&[("action", "submit")])?
            .body(body)
            .send()
            .await?;

        Ok(res)
    }

    /// 获取视频回放的 sub_info（用于下载 m3u8 playlist）, 返回 JSON 信息
    pub async fn blackboard_course_video_sub_info(
        &self,
        course_id: &str,
        sub_id: &str,
        app_id: &str,
        auth_data: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let res = self
            .http_client
            .get(VIDEO_SUB_INFO)?
            .query(&[
                ("all", "1"),
                ("course_id", course_id),
                ("sub_id", sub_id),
                ("with_sub_data", "1"),
                ("app_id", app_id),
                ("auth_data", auth_data),
            ])?
            .send()
            .await?;

        anyhow::ensure!(res.status().is_success(), "status not success");

        let rbody = res.text().await?;
        let value = serde_json::Value::from_str(&rbody)?;
        Ok(value)
    }

    /// 利用 [`convert_uri`] 将 uri 自动补全，然后发送请求.
    pub async fn get_by_uri(&self, uri: &str) -> anyhow::Result<cyper::Response> {
        let res = self
            .http_client
            .get(convert_uri(uri)?)
            .context("create request failed")?
            .send()
            .await?;
        Ok(res)
    }

    /// 发送请求给 `https://course.pku.edu.cn/{path_and_query}`, 返回页面 HTML
    pub async fn page_by_uri(&self, uri: &str) -> anyhow::Result<Html> {
        let res = self.get_by_uri(uri).await?;

        anyhow::ensure!(res.status().is_success(), "status not success");

        let rbody = res.text().await?;
        let dom = scraper::Html::parse_document(&rbody);
        Ok(dom)
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
