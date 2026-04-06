//! 校内信息门户 API

use super::LowLevelClient;

pub const PORTAL_APP_ID: &str = "portalPublicQuery";
pub const PORTAL_REDIR: &str = "https://portal.pku.edu.cn/publicQuery/ssoLogin.do";
pub const PORTAL_HOME: &str = "https://portal.pku.edu.cn/publicQuery/";
pub const PORTAL_MY_COURSE_TABLE_XNDXQ_LIST: &str =
    "https://portal.pku.edu.cn/publicQuery/ctrl/topic/myCourseTable/getXndXqList.do";
pub const PORTAL_MY_COURSE_TABLE_INFO: &str =
    "https://portal.pku.edu.cn/publicQuery/ctrl/topic/myCourseTable/getCourseInfo.do";

impl LowLevelClient {
    /// 使用 OAuth 登录门户系统
    pub async fn portal_login(&self, username: &str, password: &str) -> anyhow::Result<()> {
        let token = self
            .oauth_login(PORTAL_APP_ID, username, password, PORTAL_REDIR)
            .await?;

        log::debug!("iaaa oauth token for portal {username}: {token}");

        // 门户的 SSO 登录 - 跟随重定向
        let _res = self
            .http_client
            .get(PORTAL_REDIR)?
            .query(&[("token", &token)])?
            .send()
            .await?;

        // 门户登录后会重定向，不检查状态码
        // 尝试访问主页验证登录是否成功
        let verify = self.http_client.get(PORTAL_HOME)?.send().await?;

        log::debug!("portal homepage status: {}", verify.status());

        // 如果返回 200 说明登录成功
        anyhow::ensure!(
            verify.status().is_success(),
            "portal login verification failed"
        );

        Ok(())
    }

    /// 获取个人课表 - 学年学期列表
    pub async fn portal_my_course_table_xndxq_list(&self) -> anyhow::Result<String> {
        let res = self
            .http_client
            .get(PORTAL_MY_COURSE_TABLE_XNDXQ_LIST)?
            .send()
            .await?;

        anyhow::ensure!(
            res.status().is_success(),
            "failed to fetch xndxq list: {}",
            res.status()
        );

        let rbody = res.text().await?;
        log::debug!("xndxq list response: {}", rbody);

        Ok(rbody)
    }

    /// 获取个人课表 - 课程信息
    pub async fn portal_my_course_table_info(&self, xndxq: &str) -> anyhow::Result<String> {
        let res = self
            .http_client
            .get(PORTAL_MY_COURSE_TABLE_INFO)?
            .query(&[("xndxq", xndxq)])?
            .send()
            .await?;

        anyhow::ensure!(
            res.status().is_success(),
            "failed to fetch course info: {}",
            res.status()
        );

        let rbody = res.text().await?;
        log::debug!("course info response: {}", rbody);

        Ok(rbody)
    }

    /// 获取个人课表（使用GET请求）- 旧版，保留备用
    pub async fn portal_my_course_table_get(
        &self,
        _year: &str,
        _term: &str,
    ) -> anyhow::Result<String> {
        // 先获取学年学期列表
        let xndxq_list = self.portal_my_course_table_xndxq_list().await?;
        let xndxq_json: serde_json::Value = serde_json::from_str(&xndxq_list)?;

        // 获取当前学年学期
        let xndxq = xndxq_json
            .get("nowXnxq")
            .and_then(|n| n.get("xndxq"))
            .and_then(|x| x.as_str())
            .ok_or_else(|| anyhow::anyhow!("无法获取当前学年学期"))?;

        log::debug!("using xndxq: {}", xndxq);

        // 获取课程信息
        self.portal_my_course_table_info(xndxq).await
    }
}
