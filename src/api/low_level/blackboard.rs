use super::*;

pub const OAUTH_REDIR: &str =
    "http://course.pku.edu.cn/webapps/bb-sso-BBLEARN/execute/authValidate/campusLogin";
pub const SSO_LOGIN: &str =
    "https://course.pku.edu.cn/webapps/bb-sso-BBLEARN/execute/authValidate/campusLogin";
pub const BB_HOME: &str = "https://course.pku.edu.cn/webapps/portal/execute/tabs/tabAction";
pub const BB_LOGIN: &str = "https://course.pku.edu.cn/webapps/login/";
pub const BB_CONTENT_FILE: &str =
    "https://course.pku.edu.cn/webapps/blackboard/execute/content/file";
pub const COURSE_INFO: &str = "https://course.pku.edu.cn/webapps/blackboard/execute/announcement";
pub const UPLOAD_ASSIGNMENT: &str = "https://course.pku.edu.cn/webapps/assignment/uploadAssignment";
pub const LIST_CONTENT: &str =
    "https://course.pku.edu.cn/webapps/blackboard/content/listContent.jsp";
pub const VIDEO_LIST: &str =
    "https://course.pku.edu.cn/webapps/bb-streammedia-hqy-BBLEARN/videoList.action";
pub const VIDEO_SUB_INFO: &str =
    "https://yjapise.pku.edu.cn/courseapi/v2/schedule/get-sub-info-by-auth-data";
pub const INLINE_VIEW: &str = "https://course.pku.edu.cn/webapps/assignment/inlineView";
pub const GRADE_ASSIGNMENT: &str =
    "https://course.pku.edu.cn/webapps/assignment/gradeAssignmentRedirector";
pub const LOAD_RECONCILE_DATA: &str =
    "https://course.pku.edu.cn/webapps/gradebook/controller/loadReconcileData";
pub const RECONCILE_GRADES: &str =
    "https://course.pku.edu.cn/webapps/gradebook/controller/reconcileGrades";
pub const SAVE_RECONCILE_GRADE: &str =
    "https://course.pku.edu.cn/webapps/gradebook/controller/saveReconcileGrade";
pub const SET_ATTEMPT_IGNORED: &str =
    "https://course.pku.edu.cn/webapps/gradebook/do/instructor/setAttemptIgnored";

#[derive(Debug)]
pub struct BlackboardUnautherizedError;

impl std::error::Error for BlackboardUnautherizedError {}

impl std::fmt::Display for BlackboardUnautherizedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "blackboard login not authorized")
    }
}

impl LowLevelClient {
    pub async fn bb_login_require_otp(&self, username: &str) -> anyhow::Result<bool> {
        let data = self.iaaa_is_mobile_authen("blackboard", username).await?;
        Ok(data.authen_mode == "OTP")
    }

    /// 使用 OAuth login 返回的 token 登录教学网。登录状态会记录在 client cookie 中，无需返回值.
    pub async fn bb_login(
        &self,
        username: &str,
        password: &str,
        otp_code: &str,
    ) -> anyhow::Result<()> {
        let data = self.iaaa_is_mobile_authen("blackboard", username).await?;

        if data.is_no() {
            log::info!("unprotected login is allowed");
        } else {
            log::warn!("unsupported login context: {data:?}")
        }

        let token = self
            .iaaa_oauth_login("blackboard", username, password, otp_code, OAUTH_REDIR)
            .await?;

        log::debug!("iaaa oauth token for {username}: {token}");

        let mut rng = rand::rng();

        let _rand: f64 = rng.sample(rand::distr::Open01);
        let _rand = format!("{_rand:.20}");

        let mut res = self
            .http_client
            .get(SSO_LOGIN)?
            .query(&[("_rand", _rand.as_str()), ("token", &token)])?
            .send()
            .await?;

        // It seems that multiple redirections are possible during sso login.
        while let Ok(url) = extract_redirect_url(&res) {
            log::debug!("sso login redirected to {url}");
            res = self.get_by_uri(url).await?;
        }

        anyhow::ensure!(
            res.status().is_success(),
            "sso login not success: {}",
            res.status()
        );

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

        if extract_redirect_url(&res).ok() == Some(BB_LOGIN) {
            anyhow::bail!(BlackboardUnautherizedError);
        }
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
                format!("multipart/form-data; boundary={boundary}"),
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

    /// 获取视频回放的 sub_info（用于下载 m3u8 playlist）, 返回 response body
    pub async fn bb_course_video_sub_info(
        &self,
        course_id: &str,
        sub_id: &str,
        app_id: &str,
        auth_data: &str,
    ) -> anyhow::Result<String> {
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
        Ok(rbody)
    }

    pub async fn bb_course_content_file_uri(
        &self,
        course_id: &str,
        content_id: &str,
    ) -> anyhow::Result<String> {
        let res = self
            .http_client
            .get(BB_CONTENT_FILE)?
            .query(&[
                ("cmd", "view"),
                ("content_id", content_id),
                ("course_id", course_id),
                ("launch_in_new", "true"),
            ])?
            .send()
            .await?;

        let body = res.text().await?;
        let re = regex::Regex::new(r#"document.location = '(.*?)';"#).unwrap();
        let caps = re.captures(&body).unwrap();
        let loc = caps.get(1).unwrap().as_str();
        log::debug!("redirected to {loc}");

        let res = self.get_by_uri(loc).await?;
        let loc = extract_redirect_url(&res)?;
        Ok(loc.to_owned())
    }
}

// REST API 支持
impl LowLevelClient {
    /// 发送 GET 请求到 REST API 并解析 JSON 响应
    pub async fn api_get<T: serde::de::DeserializeOwned>(&self, url: &str) -> anyhow::Result<T> {
        let res = self.http_client.get(url)?.send().await?;

        anyhow::ensure!(
            res.status().is_success(),
            "API request failed: {}",
            res.status()
        );

        let rbody = res.text().await?;
        let data: T = serde_json::from_str(&rbody)?;
        Ok(data)
    }

    /// 获取 inlineView 返回的 JSON 文本（包含 downloadUrl）
    pub async fn bb_inline_view(
        &self,
        file_id: &str,
        attempt_id: &str,
        course_id: &str,
    ) -> anyhow::Result<String> {
        let res = self
            .http_client
            .get(INLINE_VIEW)?
            .query(&[
                ("file_id", file_id),
                ("attempt_id", attempt_id),
                ("course_id", course_id),
            ])?
            .send()
            .await?;

        anyhow::ensure!(
            res.status().is_success(),
            "inlineView failed: {}",
            res.status()
        );

        Ok(res.text().await?)
    }

    /// 获取 TA 评分页面 HTML，用于提取 file_id
    pub async fn bb_grading_page(
        &self,
        outcome_definition_id: &str,
        course_id: &str,
        attempt_id: &str,
        course_membership_id: &str,
    ) -> anyhow::Result<Html> {
        let res = self
            .http_client
            .get(GRADE_ASSIGNMENT)?
            .query(&[
                ("outcomeDefinitionId", outcome_definition_id),
                ("course_id", course_id),
                ("attempt_id", attempt_id),
                ("courseMembershipId", course_membership_id),
                ("mode", "inline"),
                ("currentAttemptIndex", "0"),
                ("numAttempts", "1"),
                ("sequenceId", &format!("{course_id}_0")),
            ])?
            .send()
            .await?;

        anyhow::ensure!(
            res.status().is_success(),
            "grading page failed: {}",
            res.status()
        );

        let rbody = res.text().await?;
        Ok(scraper::Html::parse_document(&rbody))
    }

    /// 下载提交文件（使用 inlineView 返回的 downloadUrl 的相对路径部分）
    pub async fn bb_download_attempt_file(
        &self,
        download_url: &str,
    ) -> anyhow::Result<bytes::Bytes> {
        let url = format!("https://course.pku.edu.cn{download_url}");
        let res = self.http_client.get(&url)?.send().await?;

        anyhow::ensure!(
            res.status().is_success(),
            "attempt file download failed: {}",
            res.status()
        );

        Ok(res.bytes().await?)
    }

    /// 获取 reconciliation 数据（评分复核信息）
    pub async fn bb_load_reconcile_data(
        &self,
        course_id: &str,
        column_id: &str,
    ) -> anyhow::Result<String> {
        let res = self
            .http_client
            .get(LOAD_RECONCILE_DATA)?
            .query(&[("course_id", course_id), ("id", column_id)])?
            .send()
            .await?;
        anyhow::ensure!(
            res.status().is_success(),
            "load reconcile data failed: {}",
            res.status()
        );
        Ok(res.text().await?)
    }

    /// 获取 reconcile 页面 HTML（用于提取 CSRF nonce）
    pub async fn bb_reconcile_page(
        &self,
        course_id: &str,
        column_id: &str,
    ) -> anyhow::Result<Html> {
        let res = self
            .http_client
            .get(RECONCILE_GRADES)?
            .query(&[("course_id", course_id), ("id", column_id)])?
            .send()
            .await?;
        anyhow::ensure!(
            res.status().is_success(),
            "reconcile page failed: {}",
            res.status()
        );
        let body = res.text().await?;
        Ok(scraper::Html::parse_document(&body))
    }

    /// 保存评分（登分）
    pub async fn bb_save_grade(
        &self,
        attempt_id: &str,
        gradable_item_id: &str,
        score: f64,
        course_id: &str,
        nonce: &str,
        feedback: Option<&str>,
    ) -> anyhow::Result<String> {
        let has_feedback = feedback.is_some();
        let mut params: Vec<(&str, String)> = vec![
            ("attemptId", attempt_id.to_owned()),
            ("gradableItemId", gradable_item_id.to_owned()),
            ("score", format!("{score:.2}")),
            ("hasFeedback", has_feedback.to_string()),
            ("showStagedFeedbackToStu", "true".to_owned()),
            ("isDetailPage", "false".to_owned()),
            ("reconcileMode", "A".to_owned()),
            ("course_id", course_id.to_owned()),
            (
                "blackboard.platform.security.NonceUtil.nonce.ajax",
                nonce.to_owned(),
            ),
        ];
        if let Some(fb) = feedback {
            params.push(("myfeedbacktext", fb.to_owned()));
        }
        let referer = format!(
            "https://course.pku.edu.cn/webapps/gradebook/controller/reconcileGrades?course_id={course_id}&id={gradable_item_id}"
        );

        let res = self
            .http_client
            .post(SAVE_RECONCILE_GRADE)?
            .header("origin", "https://course.pku.edu.cn")?
            .header("referer", &referer)?
            .header("x-requested-with", "XMLHttpRequest")?
            .header("x-prototype-version", "1.7")?
            .form(&params)?
            .send()
            .await?;
        anyhow::ensure!(
            res.status().is_success(),
            "save grade failed: {}",
            res.status()
        );
        Ok(res.text().await?)
    }

    /// 忽略某次提交（在 students 有多份提交时，忽略较早的）
    pub async fn bb_set_attempt_ignored(
        &self,
        course_id: &str,
        attempt_id: &str,
        outcome_definition_id: &str,
        course_membership_id: &str,
    ) -> anyhow::Result<()> {
        let res = self
            .http_client
            .get(SET_ATTEMPT_IGNORED)?
            .query(&[
                ("course_id", course_id),
                ("attemptId", attempt_id),
                ("outcomeDefinitionId", outcome_definition_id),
                ("courseMembershipId", course_membership_id),
            ])?
            .send()
            .await?;
        anyhow::ensure!(
            res.status().is_success(),
            "set attempt ignored failed: {}",
            res.status()
        );
        Ok(())
    }
}
