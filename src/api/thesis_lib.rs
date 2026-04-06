use crate::api::{drm_lib::DrmView, low_level::thesis_lib::CasLoginData};

use super::*;

impl Client {
    pub async fn thesis_lib(&self, username: &str, password: &str) -> anyhow::Result<ThesisLib> {
        let c = &self.0.http_client;
        let data = c.thesis_lib_login(username, password).await?;

        log::info!("logged in to thesis.lib.pku.edu.cn");
        log::debug!("login key: {}", data.login_key);
        log::debug!("token: {}", data.token);

        Ok(ThesisLib {
            client: self.clone(),
            login_data: data,
        })
    }
}

#[derive(Debug)]
pub struct ThesisLib {
    client: Client,
    login_data: CasLoginData,
}

impl ThesisLib {
    pub async fn search(&self, keyword: &str) -> anyhow::Result<SimpSearchData> {
        let text = self
            .client
            .thesis_lib_simp_search(&self.login_data.token, keyword, 1, 20)
            .await?;
        let body: low_level::thesis_lib::RespJson<SimpSearchData> = serde_json::from_str(&text)?;
        anyhow::ensure!(body.code == 200, "simpSearch failed: code {}", body.code);
        Ok(body.data)
    }

    pub async fn drm_view(&self, keyid: &str) -> anyhow::Result<DrmView> {
        let fid = self
            .thesis_lib_drm_view(&self.login_data.token, keyid)
            .await?;
        Ok(DrmView {
            client: self.client.clone(),
            fid,
        })
    }
}

impl std::ops::Deref for ThesisLib {
    type Target = Client;

    fn deref(&self) -> &Self::Target {
        &self.client
    }
}

/// One bucket in a facet under [`SimpSearchData::fact`] (`count` / `value` from the API).
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct SimpSearchFacetBucket {
    pub count: u64,
    pub value: String,
}

/// A single hit in [`SimpSearchData::array`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct SimpSearchHit {
    pub teacher_name: String,
    pub degree_year: String,
    issubpaper: String,
    pub author: String,
    pub title: String,
    pub department: String,
    hitcount: u32,
    pub degree_type: String,
    pub keyid: String,
    tenantname: String,
}

/// Parsed JSON body of `POST /md/papersearch/simpSearch` (matches the SPA response shape).
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct SimpSearchData {
    returnflag: bool,
    total: u64,
    #[serde(rename = "timedOut")]
    timed_out: bool,
    /// Facet field name → buckets (e.g. `degree_year`, `degree_type`, `tenantname`).
    fact: HashMap<String, Vec<SimpSearchFacetBucket>>,
    curpage: u32,
    array: Vec<SimpSearchHit>,
    #[serde(rename = "searchQuery")]
    search_query: String,
    #[serde(rename = "pageSize")]
    page_size: u32,
    time: u32,
    message: String,
}

impl SimpSearchData {
    pub fn items(&self) -> &[SimpSearchHit] {
        &self.array
    }
}
