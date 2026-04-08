use super::*;

pub const IAAA_IS_MOBILE_AUTHEN: &str = "https://iaaa.pku.edu.cn/iaaa/isMobileAuthen.do";
pub const IAAA_OAUTH_LOGIN: &str = "https://iaaa.pku.edu.cn/iaaa/oauthlogin.do";
#[cfg(feature = "thesislib")]
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

impl LowLevelClient {
    /// 向 [`IAAA_OAUTH_LOGIN`] 发送登录请求，并返回 token
    pub async fn iaaa_oauth_login(
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
        struct OAuthLoginData {
            success: bool,
            token: Option<String>,
            errors: Option<OAuthLoginError>,
        }
        let data: OAuthLoginData = serde_json::from_str(&rbody)
            .context("fail to parse response")
            .inspect_err(|e| {
                log::debug!("{e}");
                log::debug!("response body: {rbody}")
            })?;
        anyhow::ensure!(data.success, "oauth login not success");

        if let Some(err) = data.errors {
            return Err(err.into());
        }

        data.token.context("token not found")
    }

    pub async fn iaaa_is_mobile_authen(
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
    pub async fn iaaa_public_key(&self) -> anyhow::Result<String> {
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
}
