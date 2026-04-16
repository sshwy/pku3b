use std::collections::HashMap;

use super::*;

pub const THESISLIB_LOGIN: &str = "https://thesis.lib.pku.edu.cn/cas-pku/pku/login?url=https%3A%2F%2Fthesis.lib.pku.edu.cn%2Fhome";

pub const THESISLIB_SIMP_SEARCH: &str = "https://thesis.lib.pku.edu.cn/md/papersearch/simpSearch";

pub const THESISLIB_DRM_VIEW: &str = "https://thesis.lib.pku.edu.cn/md/docobject/drmView";

const THESISLIB_ORIGIN: &str = "https://thesis.lib.pku.edu.cn";
const THESISLIB_CAS_LOGIN: &str = "https://thesis.lib.pku.edu.cn/md/account/caslogin";

#[derive(serde::Deserialize, Debug)]
pub struct CasLoginData {
    #[serde(rename = "login-key")]
    pub login_key: String,
    pub token: String,
}

#[derive(serde::Deserialize)]
pub struct RespJson<T> {
    pub code: u32,
    pub data: T,
}

#[derive(serde::Serialize)]
struct SortField {
    fieldname: &'static str,
    orderby: &'static str,
    #[serde(rename = "type")]
    field_type: &'static str,
}

#[derive(serde::Serialize)]
struct FactField {
    #[serde(rename = "facetLimit", skip_serializing_if = "Option::is_none")]
    facet_limit: Option<&'static str>,
    #[serde(rename = "facetMinCount", skip_serializing_if = "Option::is_none")]
    facet_min_count: Option<&'static str>,
    #[serde(rename = "fielddesc", skip_serializing_if = "Option::is_none")]
    fielddesc: Option<&'static str>,
    fieldname: &'static str,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    field_type: Option<&'static str>,
}

#[derive(serde::Serialize)]
struct SearchField<'a> {
    fieldname: &'static str,
    keytype: &'static str,
    keyword: &'a str,
    relation: &'static str,
    searchtype: &'static str,
}

#[derive(serde::Serialize)]
struct SimpSearchBody<'a> {
    indexname: &'static str,
    curpage: u32,
    pagesize: u32,
    papertype: &'static str,
    newsearh: u32,
    searchtype: u32,
    sortfields: [SortField; 1],
    factfields: [FactField; 3],
    searchfields: [SearchField<'a>; 1],
}

fn simp_search_body(keyword: &str, curpage: u32, pagesize: u32) -> SimpSearchBody<'_> {
    SimpSearchBody {
        indexname: "paper",
        curpage,
        pagesize,
        papertype: "paper",
        newsearh: 0,
        searchtype: 1,
        sortfields: [SortField {
            fieldname: "addtime",
            orderby: "desc",
            field_type: "date",
        }],
        factfields: [
            FactField {
                facet_limit: Some("100"),
                facet_min_count: Some("1"),
                fielddesc: Some("DESC"),
                fieldname: "degree_year",
                field_type: Some("fieldnumber"),
            },
            FactField {
                facet_limit: Some("100"),
                facet_min_count: Some("1"),
                fielddesc: Some("DESC"),
                fieldname: "degree_type",
                field_type: Some("fieldstring"),
            },
            FactField {
                facet_limit: None,
                facet_min_count: Some("1"),
                fielddesc: None,
                fieldname: "tenantname",
                field_type: None,
            },
        ],
        searchfields: [SearchField {
            fieldname: "all",
            keytype: "ordinary",
            keyword,
            relation: "AND",
            searchtype: "match",
        }],
    }
}

impl LowLevelClient {
    pub async fn thesis_lib_login(
        &self,
        username: &str,
        password: &str,
    ) -> anyhow::Result<CasLoginData> {
        log::trace!("HTTP GET: {}", THESISLIB_LOGIN);
        let res = self.http_client.get(THESISLIB_LOGIN)?.send().await?;

        let url = extract_redirect_url(&res)?;

        // authorize
        log::trace!("Expection: redir to https url");
        let res = self.get_by_uri(url).await?;
        let url = extract_redirect_url(&res)?;

        // oauthLib.jsp
        let res = self.get_by_uri(url).await?;
        anyhow::ensure!(res.status().is_success(), "error status {}", res.status());

        let redir_url = {
            let url = url::Url::parse(url)?;
            let (_, v) = url
                .query_pairs()
                .find(|(k, _)| k == "redirectUrl")
                .ok_or(anyhow::anyhow!("no redirectUrl in url {url}"))?;
            v.to_string()
        };
        log::trace!("redirect_url: {redir_url}");

        let pubkey = self.iaaa_public_key().await?;
        let password = Self::encrypt_password(&pubkey, password)?;
        let token = self
            .iaaa_oauth_login("lib_sso", username, &password, "", &redir_url)
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
        let url = extract_redirect_url(&res)?;

        let res = self.get_by_uri(url).await?;
        let url = extract_redirect_url(&res)?;

        let res = self.get_by_uri(url).await?;
        anyhow::ensure!(res.status().is_success(), "error status {}", res.status());

        let url = url::Url::parse(url).context("parse url")?;
        // get learnid, name, vcode from url query
        let mut queries = url.query_pairs().collect::<HashMap<_, _>>();
        queries.insert("tenantcode".into(), "10001".into());

        let res = self
            .http_client
            .post(THESISLIB_CAS_LOGIN)?
            .query(&queries)?
            .send()
            .await?;
        anyhow::ensure!(res.status().is_success(), "error status {}", res.status());

        let body: RespJson<CasLoginData> = serde_json::from_str(&res.text().await?)?;
        anyhow::ensure!(body.code == 200, "cas login failed: code {}", body.code);

        Ok(body.data)
    }

    /// Simple keyword search (`POST /md/papersearch/simpSearch`), returning the raw JSON body.
    pub async fn thesis_lib_simp_search(
        &self,
        token: &str,
        keyword: &str,
        curpage: u32,
        pagesize: u32,
    ) -> anyhow::Result<String> {
        let body = simp_search_body(keyword, curpage, pagesize);
        let json = serde_json::to_vec(&body).context("serialize simpSearch body")?;

        log::trace!("POST {THESISLIB_SIMP_SEARCH}");

        let res = self
            .http_client
            .post(THESISLIB_SIMP_SEARCH)?
            .header(http::header::ACCEPT, "application/json, text/plain, */*")?
            .header(http::header::CONTENT_TYPE, "application/json")?
            .header(http::header::ORIGIN, THESISLIB_ORIGIN)?
            .header("token", token)?
            .body(json)
            .send()
            .await?;
        anyhow::ensure!(res.status().is_success(), "error status {}", res.status());

        let text = res.text().await?;
        Ok(text)
    }

    pub async fn thesis_lib_drm_view(&self, token: &str, keyid: &str) -> anyhow::Result<String> {
        log::trace!("POST {THESISLIB_DRM_VIEW} with keyid {keyid}");

        let res = self
            .http_client
            .get(THESISLIB_DRM_VIEW)?
            .query(&[
                ("keyid", keyid),
                ("isappend", "0"),
                ("onlineflag", "online"),
            ])?
            .header(http::header::ACCEPT, "application/json, text/plain, */*")?
            .header(
                http::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )?
            .header(http::header::ORIGIN, THESISLIB_ORIGIN)?
            .header("token", token)?
            .send()
            .await?;
        anyhow::ensure!(res.status().is_success(), "error status {}", res.status());

        let text = res.text().await?;
        let body: RespJson<String> = serde_json::from_str(&text)?;
        // personaliiifServlet url
        let url = body.data;
        let res = self.get_by_uri(&url).await?;
        let url = extract_redirect_url(&res)?;

        let fid = url::Url::parse(url)?
            .query_pairs()
            .find(|(k, _)| k == "fid")
            .context("no fid in url")?
            .1
            .to_string();
        Ok(fid)
    }
}

#[cfg(test)]
mod tests {
    use super::simp_search_body;

    #[test]
    fn simp_search_body_json_has_keyword_and_pagination() {
        let v = serde_json::to_value(simp_search_body("测试词", 2, 15)).unwrap();
        assert_eq!(v["curpage"], 2);
        assert_eq!(v["pagesize"], 15);
        assert_eq!(v["searchfields"][0]["keyword"], "测试词");
        assert_eq!(v["searchfields"][0]["fieldname"], "all");
        assert_eq!(v["indexname"], "paper");
    }
}
