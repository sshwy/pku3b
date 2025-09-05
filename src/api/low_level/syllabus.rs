use super::*;

pub const OAUTH_REDIR: &str = "http://elective.pku.edu.cn:80/elective2008/ssoLogin.do";
pub const SSO_LOGIN: &str = "https://elective.pku.edu.cn/elective2008/ssoLogin.do";

pub const SHOW_RESULTS: &str = "https://elective.pku.edu.cn/elective2008/edu/pku/stu/elective/controller/electiveWork/showResults.do";
pub const HELP_CONTROLLER: &str = "https://elective.pku.edu.cn/elective2008/edu/pku/stu/elective/controller/help/HelpController.jpf";

impl LowLevelClient {
    /// 使用 OAuth login 返回的 token 登录选课网。登录状态会记录在 client cookie 中，无需返回值.
    pub async fn sb_login(&self, username: &str, password: &str) -> anyhow::Result<()> {
        let value = self
            .oauth_login("syllabus", username, password, OAUTH_REDIR)
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

        // redir to http
        anyhow::ensure!(
            res.status().is_redirection(),
            "error status {}",
            res.status()
        );
        let Some(url) = res.headers().get("Location") else {
            anyhow::bail!("no Location header");
        };
        let url = url.to_str().context("Location not valid str")?;

        // redir to https
        let res = self.get_by_uri(url).await?;
        anyhow::ensure!(
            res.status().is_redirection(),
            "error status {}",
            res.status()
        );
        let Some(url) = res.headers().get("Location") else {
            anyhow::bail!("no Location header");
        };
        let url = url.to_str().context("Location not valid str")?;

        // final redir
        let res = self.get_by_uri(url).await?;
        anyhow::ensure!(res.status().is_success(), "error status {}", res.status());

        let cookies = self
            .http_client
            .cookie_value("https://elective.pku.edu.cn")?;
        log::debug!("cookies after sb login: {cookies:?}");

        Ok(())
    }

    /// 查看选课结果页面
    pub async fn sb_resultspage(&self) -> anyhow::Result<Html> {
        let res = self
            .http_client
            .get(SHOW_RESULTS)?
            .header(http::header::REFERER, HELP_CONTROLLER)?
            .send()
            .await?;

        anyhow::ensure!(res.status().is_success(), "status not success");

        let rbody = res.text().await?;
        let dom = scraper::Html::parse_document(&rbody);
        Ok(dom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[compio::test]
    async fn test_sb_login() {
        env_logger::builder()
            .filter_module("selectors::matching", log::LevelFilter::Info)
            .filter_module("html5ever::tokenizer", log::LevelFilter::Info)
            .filter_module("html5ever::tree_builder", log::LevelFilter::Info)
            .init();

        let c = LowLevelClient::new();
        let username = std::env::var("PKU3B_TEST_USERNAME").unwrap();
        let password = std::env::var("PKU3B_TEST_PASSWORD").unwrap();
        c.sb_login(&username, &password).await.unwrap();
    }
}
