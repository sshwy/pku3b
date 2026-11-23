use super::*;

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

/// 解析 [`IAAA_IS_MOBILE_AUTHEN`] 的响应。IAAA 出错时（如 E21 限频）响应中只有
/// `errors`，此时返回 [`OAuthLoginError`]，而不是报缺少字段。
fn parse_authen_data(rbody: &str) -> anyhow::Result<AuthenData> {
    #[derive(serde::Deserialize)]
    struct ErrorResponse {
        errors: OAuthLoginError,
    }
    if let Ok(res) = serde_json::from_str::<ErrorResponse>(rbody) {
        return Err(res.errors.into());
    }
    serde_json::from_str(rbody).context("fail to parse response")
}

/// 解析 [`IAAA_OAUTH_LOGIN`] 的响应并返回 token。登录失败（密码错误、令牌错误、
/// E21 限频等）时响应带 `errors`，此时返回 [`OAuthLoginError`]。
fn parse_oauth_login(rbody: &str) -> anyhow::Result<String> {
    #[derive(serde::Deserialize, Debug)]
    struct OAuthLoginData {
        success: bool,
        token: Option<String>,
        errors: Option<OAuthLoginError>,
    }
    let data: OAuthLoginData = serde_json::from_str(rbody).context("fail to parse response")?;
    if let Some(err) = data.errors {
        return Err(err.into());
    }
    anyhow::ensure!(data.success, "oauth login not success");

    data.token.context("token not found")
}

impl LowLevelClient {
    /// 向 [`IAAA_OAUTH_LOGIN`] 发送登录请求，并返回 token
    ///
    /// - `password`: 明文密码；提交前按网页端做法用 IAAA 公钥 RSA 加密（IAAA 已不接受明文）
    /// - `otp_code`: 手机令牌码，空串表示不提供
    pub async fn iaaa_oauth_login(
        &self,
        appid: &str,
        username: &str,
        password: &str,
        otp_code: &str,
        redir: &str,
    ) -> anyhow::Result<String> {
        // 与网页登录页一致（JSEncrypt，RSA PKCS#1 v1.5）：明文密码会被 IAAA 以 E01 拒绝
        let pubkey = self.iaaa_public_key().await?;
        let password = Self::encrypt_password(&pubkey, password)?;
        let res = self
            .http_client
            .post(IAAA_OAUTH_LOGIN)?
            .form(&[
                ("appid", appid),
                ("userName", username),
                ("password", password.as_str()),
                ("randCode", ""),
                ("smsCode", ""),
                ("otpCode", otp_code),
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
        parse_oauth_login(&rbody).inspect_err(|e| {
            log::debug!("{e}");
            log::debug!("response body: {rbody}")
        })
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
        parse_authen_data(&rbody)
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_authen_data_reports_iaaa_error() {
        // isMobileAuthen.do 在 IP 被限频时的响应 (#87)
        let body = r#"{"success":false,"errors":{"code":"E21","msg":"Too many attempts. Please sign in after a half hour."}}"#;
        let err = parse_authen_data(body).unwrap_err();
        let err = err.downcast::<OAuthLoginError>().unwrap();
        assert_eq!(err.code, "E21");
        assert!(err.msg.contains("Too many attempts"));
    }

    #[test]
    fn parse_oauth_login_reports_iaaa_error() {
        // 密码错误、令牌错误、限频时 success=false 且带 errors
        let body = r#"{"success":false,"errors":{"code":"E05","msg":"OTP code incorrect"}}"#;
        let err = parse_oauth_login(body).unwrap_err();
        let err = err.downcast::<OAuthLoginError>().unwrap();
        assert_eq!(err.code, "E05");
    }

    #[test]
    fn parse_oauth_login_returns_token() {
        let body = r#"{"success":true,"token":"abc"}"#;
        assert_eq!(parse_oauth_login(body).unwrap(), "abc");
    }

    #[test]
    fn parse_oauth_login_rejects_failure_without_errors() {
        let body = r#"{"success":false}"#;
        assert!(parse_oauth_login(body).is_err());
    }

    #[test]
    fn parse_authen_data_accepts_normal_response() {
        let body = r#"{"authenMode":"否","bzAuthMode":"","isBind":true,"isMobileAuthen":false,"isUnuAuth":true,"mobileMask":"","success":true}"#;
        let data = parse_authen_data(body).unwrap();
        assert_eq!(data.authen_mode, "否");
    }
}
