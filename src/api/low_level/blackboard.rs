use super::*;

pub const OAUTH_REDIR: &str =
    "http://course.pku.edu.cn/webapps/bb-sso-BBLEARN/execute/authValidate/campusLogin";
pub const SSO_LOGIN: &str =
    "https://course.pku.edu.cn/webapps/bb-sso-BBLEARN/execute/authValidate/campusLogin";
pub const BB_HOME: &str = "https://course.pku.edu.cn/webapps/portal/execute/tabs/tabAction";
pub const COURSE_INFO: &str = "https://course.pku.edu.cn/webapps/blackboard/execute/announcement";
pub const UPLOAD_ASSIGNMENT: &str = "https://course.pku.edu.cn/webapps/assignment/uploadAssignment";
pub const LIST_CONTENT: &str =
    "https://course.pku.edu.cn/webapps/blackboard/content/listContent.jsp";
pub const VIDEO_LIST: &str =
    "https://course.pku.edu.cn/webapps/bb-streammedia-hqy-BBLEARN/videoList.action";
pub const VIDEO_SUB_INFO: &str =
    "https://yjapise.pku.edu.cn/courseapi/v2/schedule/get-sub-info-by-auth-data";

impl LowLevelClient {
    /// 使用 OAuth login 返回的 token 登录教学网。登录状态会记录在 client cookie 中，无需返回值.
    pub async fn bb_login(&self, username: &str, password: &str) -> anyhow::Result<()> {
        let value = self
            .oauth_login("blackboard", username, password, OAUTH_REDIR)
            .await?;
        let token = value
            .as_object()
            .context("value not an object")?
            .get("token")
            .context("password not correct")?
            .as_str()
            .context("property 'token' not string")?
            .to_owned();

        log::debug!("iaaa oauth token for {username}: {token}");

        let mut rng = rand::rng();

        let _rand: f64 = rng.sample(rand::distr::Open01);
        let _rand = format!("{_rand:.20}");

        let res = self
            .http_client
            .get(SSO_LOGIN)?
            .query(&[("_rand", _rand.as_str()), ("token", &token)])?
            .send()
            .await?;
        anyhow::ensure!(res.status().is_success(), "status not success");

        Ok(())
    }

    /// 获取教学网主页内容 ([`BB_HOME`]), 返回 HTML 文档
    pub async fn bb_homepage(&self) -> anyhow::Result<Html> {
        let res = self
            .http_client
            .get(BB_HOME)?
            .query(&[("tab_tab_group_id", "_1_1")])?
            .send()
            .await?;

        anyhow::ensure!(res.status().is_success(), "status not success");

        let rbody = res.text().await?;
        let dom = scraper::Html::parse_document(&rbody);
        Ok(dom)
    }

    /// 根据课程的 key 获取课程主页内容 ([`COURSE_INFO`])
    pub async fn bb_coursepage(&self, key: &str) -> anyhow::Result<Html> {
        let res = self
            .http_client
            .get(COURSE_INFO)?
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

    /// 根据 content_id 和 course_id 获取课程内容列表页面（包含作业、公告和一些其他东西）
    pub async fn bb_course_content_page(
        &self,
        course_id: &str,
        content_id: &str,
    ) -> anyhow::Result<Html> {
        let res = self
            .http_client
            .get(LIST_CONTENT)?
            .query(&[("content_id", content_id), ("course_id", course_id)])?
            .send()
            .await?;

        anyhow::ensure!(res.status().is_success(), "status not success");

        let rbody = res.text().await?;
        let dom = scraper::Html::parse_document(&rbody);
        Ok(dom)
    }

    /// 根据 content_id 和 course_id 获取作业上传页面的信息.
    pub async fn bb_course_assignment_uploadpage(
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
    pub async fn bb_course_assignment_viewpage(
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
    pub async fn bb_course_assignment_uploaddata(
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

    /// 根据 course_id 获取回放列表页面内容.
    pub async fn bb_course_video_list(&self, course_id: &str) -> anyhow::Result<Html> {
        let res = self
            .http_client
            .get(VIDEO_LIST)?
            .query(&[
                ("sortDir", "ASCENDING"),
                ("numResults", "100"), // 一门课一般不会有超过 100 条回放
                ("editPaging", "false"),
                ("course_id", course_id),
                ("mode", "view"),
                ("startIndex", "0"),
            ])?
            .send()
            .await?;

        anyhow::ensure!(res.status().is_success(), "status not success");

        let rbody = res.text().await?;
        let dom = scraper::Html::parse_document(&rbody);
        Ok(dom)
    }

    /// 获取视频回放的 sub_info（用于下载 m3u8 playlist）, 返回 JSON 信息
    pub async fn bb_course_video_sub_info(
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[compio::test]
    async fn test_bb_login() {
        let c = LowLevelClient::new();
        let username = std::env::var("PKU3B_TEST_USERNAME").unwrap();
        let password = std::env::var("PKU3B_TEST_PASSWORD").unwrap();
        c.bb_login(&username, &password).await.unwrap();
    }
}
