use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::Context;

use super::{Client, ClientInner, low_level};

/// Builder for [`Client`].
#[derive(Clone, Default)]
pub struct ClientBuilder {
    cache_ttl: Option<Duration>,
    download_artifact_ttl: Option<Duration>,
    http_client: Option<low_level::LowLevelClient>,
    cookie_restore_path: Option<PathBuf>,
}

impl ClientBuilder {
    pub fn cache_ttl(mut self, cache_ttl: Option<Duration>) -> Self {
        self.cache_ttl = cache_ttl;
        self
    }

    pub fn download_artifact_ttl(mut self, download_artifact_ttl: Option<Duration>) -> Self {
        self.download_artifact_ttl = download_artifact_ttl;
        self
    }

    pub fn cookie_restore_path(mut self, cookie_restore_path: Option<impl AsRef<Path>>) -> Self {
        self.cookie_restore_path = cookie_restore_path.map(|p| p.as_ref().to_path_buf());
        self
    }

    pub async fn build(self) -> anyhow::Result<Client> {
        log::info!("Cache TTL: {:?}", self.cache_ttl);
        log::info!("Download Artifact TTL: {:?}", self.download_artifact_ttl);

        let http_client = self
            .http_client
            .unwrap_or_else(low_level::LowLevelClient::new);

        if let Some(path) = &self.cookie_restore_path
            && path.exists()
        {
            log::debug!("loading cookies from {}", path.display());
            if let Err(e) = http_client
                .load_set_cookies(path)
                .await
                .context("load cookies")
            {
                log::error!("{e}");
            }
        }

        Ok(Client(
            ClientInner {
                http_client,
                cache_ttl: self.cache_ttl,
                download_artifact_ttl: self.download_artifact_ttl,
                cookie_restore_path: self.cookie_restore_path,
            }
            .into(),
        ))
    }
}
