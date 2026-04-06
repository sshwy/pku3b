use super::*;
pub const DRM_PDFINDEX: &str = "https://drm.lib.pku.edu.cn/pdfindex1.jsp";
pub const DRM_JUMP_SERVLET: &str = "https://drm.lib.pku.edu.cn/jumpServlet";

impl LowLevelClient {
    pub async fn drm_lib_pdfindex(&self, fid: &str) -> anyhow::Result<Html> {
        let res = self
            .http_client
            .get(DRM_PDFINDEX)?
            .query(&[("fid", fid)])?
            .send()
            .await?;

        anyhow::ensure!(res.status().is_success(), "status not success");

        let rbody = res.text().await?;
        log::trace!("drm_lib_pdfindex response: {rbody}");
        let dom = scraper::Html::parse_document(&rbody);
        Ok(dom)
    }

    pub async fn drm_lib_pdfpage(
        &self,
        fid: &str,
        filename: &str,
        page: u32,
    ) -> anyhow::Result<String> {
        log::trace!("get pdf page {page} of {filename} for {fid}");
        let res = self
            .http_client
            .get(DRM_JUMP_SERVLET)?
            .query(&[
                ("page", page.to_string().as_str()),
                ("fid", fid),
                ("userid", ""),
                ("filename", filename),
                ("visitid", ""),
            ])?
            .send()
            .await?;

        anyhow::ensure!(res.status().is_success(), "status not success");

        let rbody = res.text().await?;
        Ok(rbody)
    }
}
