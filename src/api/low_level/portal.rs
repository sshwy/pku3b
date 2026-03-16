//! 校内信息门户 API

use super::LowLevelClient;

pub const PORTAL_REDIR: &str = "https://portal.pku.edu.cn/publicQuery/ssoLogin.do";
pub const PORTAL_COURSE_SCHEDULE_API: &str = "https://portal.pku.edu.cn/publicQuery/ctrl/common/courseQuery/retrCourseScheduleList.do";

impl LowLevelClient {
    /// 使用 OAuth 登录门户系统
    pub async fn portal_login(
        &self,
        username: &str,
        password: &str,
    ) -> anyhow::Result<()> {
        let token = self
            .oauth_login("portalPublicQuery", username, password, PORTAL_REDIR)
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
        let verify = self
            .http_client
            .get("https://portal.pku.edu.cn/publicQuery/")?
            .send()
            .await?;
        
        log::debug!("portal homepage status: {}", verify.status());
        
        // 如果返回 200 说明登录成功
        anyhow::ensure!(verify.status().is_success(), "portal login verification failed");

        Ok(())
    }

    /// 查询院系列表
    pub async fn portal_get_dept_list(&self,
    ) -> anyhow::Result<String> {
        let res = self
            .http_client
            .get("https://portal.pku.edu.cn/publicQuery/ctrl/common/util/retrJxDeptList.do")?
            .send()
            .await?;

        anyhow::ensure!(res.status().is_success(), "failed to fetch dept list: {}", res.status());

        let rbody = res.text().await?;
        log::debug!("dept list response: {}", rbody);
        
        Ok(rbody)
    }

    /// 获取课表数据（学生课表）
    /// 
    /// 参数：
    /// - year: 学年，如 "2024-2025"
    /// - term: 学期，如 "1"（秋季）、"2"（春季）
    /// - dept_id: 院系ID（从院系列表获取）
    /// - course_schedule_type: 课表类型（如 "student"）
    /// - course_type: 课程类型（可选）
    pub async fn portal_course_schedule(
        &self,
        year: &str,
        term: &str,
        dept_id: &str,
        course_schedule_type: &str,
        course_type: Option<&str>,
    ) -> anyhow::Result<String> {
        // 构建请求体
        let mut data = serde_json::json!({
            "year": year,
            "term": term,
            "deptId": dept_id,
            "courseScheduleType": course_schedule_type,
        });
        
        if let Some(ct) = course_type {
            data["courseType"] = serde_json::Value::String(ct.to_string());
        }

        let body_json = data.to_string();

        let res = self
            .http_client
            .post(PORTAL_COURSE_SCHEDULE_API)?
            .header("Content-Type", "application/json")?
            .body(body_json)
            .send()
            .await?;

        anyhow::ensure!(res.status().is_success(), "failed to fetch course schedule: {}", res.status());

        let rbody = res.text().await?;
        log::debug!("course schedule response: {}", rbody);
        
        Ok(rbody)
    }

    /// 获取当前学期的课表（简化版）
    /// 
    /// 需要先获取院系ID和课表类型
    pub async fn portal_course_schedule_current(
        &self,
    ) -> anyhow::Result<String> {
        // 先获取院系列表
        let dept_list_json = self.portal_get_dept_list().await?;
        let dept_list: serde_json::Value = serde_json::from_str(&dept_list_json)?;
        
        // 获取第一个院系ID
        let dept_id = dept_list
            .get("rows")
            .and_then(|r| r.as_array())
            .and_then(|arr| arr.first())
            .and_then(|first| first.get("id"))
            .and_then(|id| id.as_str())
            .ok_or_else(|| anyhow::anyhow!("无法获取院系ID"))?;
        
        log::debug!("using dept_id: {}", dept_id);
        
        // 获取当前学年学期（这里简化处理，实际需要计算）
        let year = "2025-2026";
        let term = "2"; // 春季学期
        
        self.portal_course_schedule(year, term, dept_id, "student", None).await
    }

    /// 获取个人课表 - 学年学期列表
    pub async fn portal_my_course_table_xndxq_list(
        &self,
    ) -> anyhow::Result<String> {
        let res = self
            .http_client
            .get("https://portal.pku.edu.cn/publicQuery/ctrl/topic/myCourseTable/getXndXqList.do")?
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
    pub async fn portal_my_course_table_info(
        &self,
        xndxq: &str,
    ) -> anyhow::Result<String> {
        let res = self
            .http_client
            .get("https://portal.pku.edu.cn/publicQuery/ctrl/topic/myCourseTable/getCourseInfo.do")?
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
