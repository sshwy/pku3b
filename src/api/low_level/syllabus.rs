use super::*;

pub const OAUTH_REDIR: &str = "http://elective.pku.edu.cn:80/elective2008/ssoLogin.do";
pub const SSO_LOGIN: &str = "https://elective.pku.edu.cn/elective2008/ssoLogin.do";

pub const SHOW_RESULTS: &str = "https://elective.pku.edu.cn/elective2008/edu/pku/stu/elective/controller/electiveWork/showResults.do";
pub const HELP_CONTROLLER: &str = "https://elective.pku.edu.cn/elective2008/edu/pku/stu/elective/controller/help/HelpController.jpf";
pub const SUPPLEMENT: &str = "https://elective.pku.edu.cn/elective2008/edu/pku/stu/elective/controller/supplement/supplement.jsp";
pub const SUPPLY_CANCEL: &str = "https://elective.pku.edu.cn/elective2008/edu/pku/stu/elective/controller/supplement/SupplyCancel.do";
pub const DRAW_SERVLET: &str = "https://elective.pku.edu.cn/elective2008/DrawServlet";
pub const VALIDATE: &str = "https://elective.pku.edu.cn/elective2008/edu/pku/stu/elective/controller/supplement/validate.do";

impl LowLevelClient {
    /// 使用 OAuth login 返回的 token 登录选课网。登录状态会记录在 client cookie 中，无需返回值.
    pub async fn sb_login(&self, username: &str, password: &str) -> anyhow::Result<()> {
        let token = self
            .iaaa_oauth_login("syllabus", username, password, OAUTH_REDIR)
            .await?;

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

        log::trace!("Expection: redir to http url");
        if res.status() == http::StatusCode::OK {
            // Check for dual degree selection page
            let res = res.text().await?;
            let res = Html::parse_document(res.as_str());
            let selector1 =
                scraper::Selector::parse("#div1").expect("Static CSS selector should parse");
            let selector2 =
                scraper::Selector::parse("#div2").expect("Static CSS selector should parse");

            fn get_div_content_by_selector(
                html: &Html,
                selector: &scraper::Selector,
            ) -> Option<String> {
                html.select(selector)
                    .next()
                    .map(|element_ref| element_ref.inner_html())
            }
            let text1 = get_div_content_by_selector(&res, &selector1).ok_or(anyhow::anyhow!(
                "Dual degree selection page expected but not detected. Is the page updated?"
            ))?;
            let text2 = get_div_content_by_selector(&res, &selector2).ok_or(anyhow::anyhow!(
                "Dual degree selection page expected but not detected. Is the page updated?"
            ))?;
            if text1 == "主 修" && text2 == "辅 双" {
                anyhow::bail!(
                    "major or minor degree unspecified. Use `pku3b s -d major|minor` to specify."
                );
            } else {
                anyhow::bail!("Unexpected HTTP 200. Redirect expected.");
            }
        }

        let url = extract_redirect_url(&res)?;

        log::trace!("Expection: redir to https url");
        let res = self.get_by_uri(url).await?;

        let url = extract_redirect_url(&res)?;

        log::trace!("final redir");
        let res = self.get_by_uri(url).await?;
        anyhow::ensure!(res.status().is_success(), "error status {}", res.status());

        Ok(())
    }

    /// 使用 OAuth login 返回的 token 登录选课网。登录状态会记录在 client cookie 中，无需返回值.
    pub async fn sb_login_dual_degree(
        &self,
        username: &str,
        password: &str,
        dual_sttp: &str,
    ) -> anyhow::Result<()> {
        let token = self
            .iaaa_oauth_login("syllabus", username, password, OAUTH_REDIR)
            .await?;

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
        anyhow::ensure!(res.status().is_success(), "error status {}", res.status());

        let body = res.text().await?;
        let re = regex::Regex::new(r"\?sida=(\S+?)&sttp=(?:bzx|bfx)").unwrap();
        let sida = re
            .captures(&body)
            .context("no sida in response")?
            .get(1)
            .context("no sida in response")?
            .as_str();
        anyhow::ensure!(sida.len() == 32, "invalid sida {}", sida);

        let res = self
            .http_client
            .get(SSO_LOGIN)?
            .query(&[("sida", sida), ("sttp", dual_sttp)])?
            .send()
            .await?;

        log::trace!("redir to http");
        let url = extract_redirect_url(&res)?;

        log::trace!("redir to https");
        let res = self.get_by_uri(url).await?;
        let url = extract_redirect_url(&res)?;

        log::trace!("final redir");
        let res = self.get_by_uri(url).await?;
        anyhow::ensure!(res.status().is_success(), "error status {}", res.status());

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

    /// 查看补退选首页
    pub async fn sb_supplycancelpage(&self, username: &str) -> anyhow::Result<Html> {
        let res = self
            .http_client
            .get(SUPPLY_CANCEL)?
            .query(&[("xh", username)])?
            .header(http::header::REFERER, HELP_CONTROLLER)?
            .header(http::header::CACHE_CONTROL, "max-age=0")?
            .send()
            .await?;

        anyhow::ensure!(res.status().is_success(), "status not success");

        let rbody = res.text().await?;
        let dom = scraper::Html::parse_document(&rbody);
        Ok(dom)
    }

    /// 查看补退选页面，page=0 表示第一页
    pub async fn sb_supplementpage(&self, username: &str, page: usize) -> anyhow::Result<Html> {
        let res = self
            .http_client
            .get(SUPPLEMENT)?
            .query(&[
                ("xh", username),
                ("netui_row", &format!("electableListGrid;{}", page * 20)),
            ])?
            .header(http::header::REFERER, SUPPLY_CANCEL)?
            .header(http::header::CACHE_CONTROL, "max-age=0")?
            .send()
            .await?;

        anyhow::ensure!(res.status().is_success(), "status not success");

        let rbody = res.text().await?;
        let dom = scraper::Html::parse_document(&rbody);
        Ok(dom)
    }

    /// 获取验证码图片内容 (JPEG 格式)
    pub async fn sb_draw_servlet(&self) -> anyhow::Result<bytes::Bytes> {
        let mut rng = rand::rng();
        let _rand: f64 = rng.sample(rand::distr::Open01);
        let _rand = format!("{_rand:.20}");

        let res = self
            .http_client
            .get(DRAW_SERVLET)?
            .query(&[("Rand", &_rand)])?
            .header(http::header::REFERER, SUPPLY_CANCEL)?
            .send()
            .await?;

        anyhow::ensure!(res.status().is_success(), "status not success");
        let ct = res
            .headers()
            .get(http::header::CONTENT_TYPE)
            .context("no Content-Type header")?;
        anyhow::ensure!(ct == "image/jpeg", "Content-Type not image/jpeg: {ct:?}");

        let bytes = res.bytes().await?;
        Ok(bytes)
    }

    /// 发送验证码，返回验证结果。2 表示成功，1 表示未填写，0 表示不正确
    pub async fn sb_send_validation(&self, username: &str, code: &str) -> anyhow::Result<i32> {
        let mut rng = rand::rng();
        let _rand: f64 = rng.sample(rand::distr::Open01);
        let _rand = format!("{_rand:.20}");

        let body = format!("xh={username}&validCode={code}");

        let res = self
            .http_client
            .post(VALIDATE)?
            .header(http::header::REFERER, SUPPLY_CANCEL)?
            .header(
                http::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded; charset=UTF-8",
            )?
            .body(body)
            .send()
            .await?;

        anyhow::ensure!(res.status().is_success(), "status not success");

        #[derive(serde::Deserialize)]
        struct ResData {
            valid: String,
        }

        let content = res.text().await?;
        let res_data: ResData =
            serde_json::from_str(&content).context("fail to parse response json")?;
        Ok(res_data.valid.parse()?)
    }

    pub async fn sb_elect_by_url(&self, url: &str) -> anyhow::Result<Html> {
        let res = self
            .http_client
            .get(url)?
            .header(http::header::REFERER, SUPPLY_CANCEL)?
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
