use super::*;
use std::{collections::BTreeMap, sync::RwLock};

pub struct DrmView {
    pub(crate) client: Client,
    pub(crate) fid: String,
}

impl DrmView {
    pub async fn get_pdf(&self) -> anyhow::Result<DrmLibPdf> {
        let fid = &self.fid;
        log::info!("fetching pdf index {fid}");
        let dom = self.client.drm_lib_pdfindex(fid).await?;

        let parse_u32_by_id = |id: &str| -> anyhow::Result<u32> {
            let value = dom
                .select(&Selector::parse(id).unwrap())
                .next()
                .with_context(|| format!("{id} not found"))?
                .attr("value")
                .with_context(|| format!("{id} value not found"))?;
            log::trace!("parse {id} = {value}");
            Ok(value.parse::<u32>()?)
        };

        let startpage = parse_u32_by_id("#startpage")?;
        let endpage = parse_u32_by_id("#endpage")?;
        anyhow::ensure!(
            endpage >= startpage,
            "invalid page range: start={startpage}, end={endpage}"
        );

        let filename = dom
            .select(&Selector::parse("#filename").unwrap())
            .next()
            .context("#filename not found")?
            .attr("value")
            .context("#filename value not found")?
            .to_owned();

        log::info!("fid: {fid}");
        log::info!("filename: {filename}");

        Ok(DrmLibPdf {
            client: self.client.clone(),
            fid: fid.to_string(),
            filename,
            startpage,
            endpage,
            urlmap_cache: Default::default(),
        })
    }
}
pub struct DrmLibPdf {
    client: Client,
    fid: String,
    filename: String,
    startpage: u32,
    endpage: u32,
    urlmap_cache: RwLock<BTreeMap<u32, String>>,
}

impl DrmLibPdf {
    pub fn maxpage(&self) -> u32 {
        self.endpage - self.startpage
    }

    async fn get_pdf_page_info(&self, page: u32) -> anyhow::Result<PdfPageInfo> {
        let info = self
            .client
            .drm_lib_pdfpage(&self.fid, &self.filename, page)
            .await?;

        let info: PdfPageInfo = serde_json::from_str(&info)
            .with_context(|| format!("parse drm page info failed, page={page}"))?;

        Ok(info)
    }

    fn get_src(&self, id: u32) -> Option<String> {
        self.urlmap_cache.read().unwrap().get(&id).cloned()
    }

    fn insert_pair(&self, id: u32, src: String) {
        self.urlmap_cache.write().unwrap().insert(id, src);
    }

    async fn update_urlmap_cache(&self, id: u32) -> anyhow::Result<()> {
        if self.get_src(id).is_some() {
            return Ok(());
        }
        let info = self.get_pdf_page_info(self.maxpage().min(id + 1)).await?;
        for item in info.list {
            let Ok(id) = item.id.parse::<u32>() else {
                continue;
            };
            self.insert_pair(id, item.src);
        }
        Ok(())
    }

    async fn _get_page_image(&self, id: u32) -> anyhow::Result<bytes::Bytes> {
        log::info!("fetching page image {id}");
        self.update_urlmap_cache(id).await?;
        let src = self
            .get_src(id)
            .with_context(|| format!("page id not found: {id}"))?;
        let resp = self.client.get_by_uri(&src).await?;
        anyhow::ensure!(
            resp.status().is_success(),
            "failed to fetch page image {src}, status={}",
            resp.status()
        );

        let bytes = resp.bytes().await?;
        Ok(bytes)
    }
    pub async fn get_page_image(&self, id: u32) -> anyhow::Result<bytes::Bytes> {
        with_cache_bytes(
            &format!("DrmLibPdf::get_page_image_{}_{id}", self.fid),
            self.client.download_artifact_ttl(),
            self._get_page_image(id),
        )
        .await
    }
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct PdfPageInfo {
    list: Vec<PdfPageInfoItem>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct PdfPageInfoItem {
    id: String,
    src: String,
}
