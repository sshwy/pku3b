use super::*;

impl Client {
    pub async fn portal(
        &self,
        username: &str,
        password: &str,
        otp_code: &str,
    ) -> anyhow::Result<Portal> {
        let c = &self.0.http_client;

        c.portal_login(username, password, otp_code).await?;

        Ok(Portal {
            client: self.clone(),
        })
    }
}

#[derive(Debug)]
pub struct Portal {
    client: Client,
}

impl Portal {
    /// 获取个人课表
    pub async fn get_my_course_table(&self) -> anyhow::Result<String> {
        self.client
            .0
            .http_client
            .portal_my_course_table_get("", "")
            .await
    }
}
