use super::*;

pub const THESISLIB_LOGIN: &str = "https://thesis.lib.pku.edu.cn/cas-pku/pku/login?url=https%3A%2F%2Fthesis.lib.pku.edu.cn%2Fhome";

impl LowLevelClient {
    pub async fn thesis_lib_login(&self, username: &str, password: &str) -> anyhow::Result<()> {
        log::trace!("HTTP GET: {}", THESISLIB_LOGIN);
        let res = self.http_client.get(THESISLIB_LOGIN)?.send().await?;

        anyhow::ensure!(
            res.status().is_redirection(),
            "error status {}",
            res.status()
        );
        let Some(url) = res.headers().get("Location") else {
            anyhow::bail!("no Location header");
        };
        let url = url.to_str().context("Location not valid str")?;

        // authorize
        log::trace!("Expection: redir to https url");
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

        // oauthLib.jsp
        let res = self.get_by_uri(url).await?;
        anyhow::ensure!(res.status().is_success(), "error status {}", res.status());

        let redir_url = {
            let url = url::Url::parse(url)?;
            let (_, v) = url
                .query_pairs()
                .filter(|(k, _)| k == "redirectUrl")
                .next()
                .ok_or(anyhow::anyhow!("no redirectUrl in url {url}"))?;
            v.to_string()
        };
        log::trace!("redirect_url: {redir_url}");

        let pubkey = self.iaaa_public_key().await?;
        let password = self.encrypt_password(&pubkey, password)?;
        let token = self
            .oauth_login("lib_sso", username, &password, &redir_url)
            .await?;

        let mut rng = rand::rng();
        let _rand: f64 = rng.sample(rand::distr::Open01);
        let _rand = format!("{_rand:.20}");
        let mut redir_url = url::Url::parse(&redir_url)?;
        redir_url
            .query_pairs_mut()
            .append_pair("token", &token)
            .append_pair("_rand", &_rand);
        let redir_url = redir_url.to_string();
        let res = self.get_by_uri(&redir_url).await?;
        anyhow::ensure!(res.status().is_success(), "error status {}", res.status());

        let body = res.text().await?;
        let dom = scraper::Html::parse_document(&body);
        let meta_sel = scraper::Selector::parse("meta[http-equiv='refresh']").unwrap();
        let el = dom
            .select(&meta_sel)
            .next()
            .ok_or(anyhow::anyhow!("no meta[http-equiv='refresh']"))?;
        let content = el
            .attr("content")
            .ok_or(anyhow::anyhow!("no content in meta[http-equiv='refresh']"))?;

        let url = content
            .split_once(";url=")
            .ok_or(anyhow::anyhow!("no url in content"))?
            .1;
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

        let res = self.get_by_uri(url).await?;
        anyhow::ensure!(res.status().is_success(), "error status {}", res.status());

        Ok(())
    }
}
