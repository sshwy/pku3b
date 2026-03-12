mod low_level;

use anyhow::Context;
use chrono::TimeZone;
use cyper::IntoUrl;
use itertools::Itertools;
use scraper::Selector;
use std::{
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
    str::FromStr,
    sync::Arc,
};

use crate::{
    multipart, qs,
    utils::{with_cache, with_cache_bytes},
};

const ONE_HOUR: std::time::Duration = std::time::Duration::from_secs(3600);
const ONE_DAY: std::time::Duration = std::time::Duration::from_secs(3600 * 24);

struct ClientInner {
    http_client: low_level::LowLevelClient,
    cache_ttl: Option<std::time::Duration>,
    download_artifact_ttl: Option<std::time::Duration>,
}

impl std::fmt::Debug for ClientInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientInner")
            .field("cache_ttl", &self.cache_ttl)
            .field("download_artifact_ttl", &self.download_artifact_ttl)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct Client(Arc<ClientInner>);

impl std::ops::Deref for Client {
    type Target = low_level::LowLevelClient;

    fn deref(&self) -> &Self::Target {
        &self.0.http_client
    }
}

impl Client {
    pub fn new(
        cache_ttl: Option<std::time::Duration>,
        download_artifact_ttl: Option<std::time::Duration>,
    ) -> Self {
        log::info!("Cache TTL: {cache_ttl:?}");
        log::info!("Download Artifact TTL: {download_artifact_ttl:?}");

        Self(
            ClientInner {
                http_client: low_level::LowLevelClient::new(),
                cache_ttl,
                download_artifact_ttl,
            }
            .into(),
        )
    }

    pub fn new_nocache() -> Self {
        Self::new(None, None)
    }

    pub fn cache_ttl(&self) -> Option<&std::time::Duration> {
        self.0.cache_ttl.as_ref()
    }

    pub fn download_artifact_ttl(&self) -> Option<&std::time::Duration> {
        self.0.download_artifact_ttl.as_ref()
    }
}

impl Default for Client {
    fn default() -> Self {
        Self::new(Some(ONE_HOUR), Some(ONE_DAY))
    }
}

pub mod blackboard;

pub mod syllabus;

/// 根据文件扩展名返回对应的 MIME 类型
pub fn get_mime_type(extension: &str) -> &str {
    let mime_types: HashMap<&str, &str> = [
        ("html", "text/html"),
        ("htm", "text/html"),
        ("txt", "text/plain"),
        ("csv", "text/csv"),
        ("json", "application/json"),
        ("xml", "application/xml"),
        ("png", "image/png"),
        ("jpg", "image/jpeg"),
        ("jpeg", "image/jpeg"),
        ("gif", "image/gif"),
        ("bmp", "image/bmp"),
        ("webp", "image/webp"),
        ("mp3", "audio/mpeg"),
        ("wav", "audio/wav"),
        ("mp4", "video/mp4"),
        ("avi", "video/x-msvideo"),
        ("pdf", "application/pdf"),
        ("zip", "application/zip"),
        ("tar", "application/x-tar"),
        ("7z", "application/x-7z-compressed"),
        ("rar", "application/vnd.rar"),
        ("exe", "application/octet-stream"),
        ("bin", "application/octet-stream"),
    ]
    .iter()
    .cloned()
    .collect();

    mime_types
        .get(extension)
        .copied()
        .unwrap_or("application/octet-stream")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_mime_type() {
        assert_eq!(get_mime_type("html"), "text/html");
        assert_eq!(get_mime_type("png"), "image/png");
        assert_eq!(get_mime_type("mp3"), "audio/mpeg");
        assert_eq!(get_mime_type("unknown"), "application/octet-stream");
    }
}
