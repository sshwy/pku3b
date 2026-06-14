use super::{Client, Course, CourseMeta};
use crate::api::low_level;
use crate::qs;
use crate::utils::{with_cache, with_cache_bytes};
use anyhow::Context;
use cyper::IntoUrl;
use scraper::Selector;
use std::{
    hash::{Hash, Hasher},
    str::FromStr,
    sync::Arc,
};

impl Course {
    pub async fn get_video_list(&self) -> anyhow::Result<Vec<CourseVideoHandle>> {
        log::info!("fetching video list for course {}", self.meta.title());

        let videos = with_cache(
            &format!("Course::get_video_list_{}", self.meta.id),
            self.client.cache_ttl(),
            self._get_video_list(),
        )
        .await?;

        let videos = videos
            .into_iter()
            .map(|meta| {
                Ok(CourseVideoHandle {
                    client: self.client.clone(),
                    meta: meta.into(),
                    course: self.meta.clone(),
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        Ok(videos)
    }

    async fn _get_video_list(&self) -> anyhow::Result<Vec<CourseVideoMeta>> {
        let u = low_level::blackboard::VIDEO_LIST.into_url()?;
        let dom = self.client.bb_course_video_list(&self.meta.id).await?;

        let videos = dom
            .select(&Selector::parse("tbody#listContainer_databody > tr").unwrap())
            .map(|tr| {
                let title = tr
                    .child_elements()
                    .nth(0)
                    .unwrap()
                    .text()
                    .collect::<String>();
                let s = Selector::parse("span.table-data-cell-value").unwrap();
                let mut values = tr.select(&s);
                let time = values
                    .next()
                    .context("time not found")?
                    .text()
                    .collect::<String>();
                let _ = values.next().context("teacher not found")?;
                let link = values.next().context("video link not found")?;
                let a = link
                    .child_elements()
                    .next()
                    .context("video link anchor not found")?;
                let link = a
                    .value()
                    .attr("href")
                    .context("video link not found")?
                    .to_owned();

                Ok(CourseVideoMeta {
                    title,
                    time,
                    url: u.join(&link)?.to_string(),
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        Ok(videos)
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct CourseVideoMeta {
    title: String,
    time: String,
    url: String,
}

impl CourseVideoMeta {
    pub fn title(&self) -> &str {
        &self.title
    }
    pub fn time(&self) -> &str {
        &self.time
    }
}

#[derive(Debug)]
pub struct CourseVideoHandle {
    client: Client,
    meta: Arc<CourseVideoMeta>,
    course: Arc<CourseMeta>,
}

impl CourseVideoHandle {
    /// Course video identifier computed from hash.
    pub fn id(&self) -> String {
        let mut hasher = std::hash::DefaultHasher::new();
        self.course.id.hash(&mut hasher);
        self.meta.title.hash(&mut hasher);
        self.meta.time.hash(&mut hasher);
        let x = hasher.finish();
        format!("{x:x}")
    }
    pub fn meta(&self) -> &CourseVideoMeta {
        &self.meta
    }
    async fn get_iframe_url(&self) -> anyhow::Result<String> {
        let res = self.client.get_by_uri(&self.meta.url).await?;
        anyhow::ensure!(res.status().is_success(), "status not success");
        let rbody = res.text().await?;
        let dom = scraper::Html::parse_document(&rbody);
        let iframe = dom
            .select(&Selector::parse("#content iframe").unwrap())
            .next()
            .context("iframe not found")?;
        let src = iframe
            .value()
            .attr("src")
            .context("src not found")?
            .to_owned();

        let res = self.client.get_by_uri(&src).await?;
        anyhow::ensure!(res.status().as_u16() == 302, "status not 302");
        let loc = res
            .headers()
            .get("location")
            .context("location header not found")?
            .to_str()
            .context("location header not str")?
            .to_owned();

        Ok(loc)
    }

    async fn get_sub_info(&self, loc: &str) -> anyhow::Result<String> {
        let qs = qs::Query::from_str(loc).context("parse loc qs failed")?;
        let course_id = qs
            .get("course_id")
            .context("course_id not found")?
            .to_owned();
        let sub_id = qs.get("sub_id").context("sub_id not found")?.to_owned();
        let app_id = qs.get("app_id").context("app_id not found")?.to_owned();
        let auth_data = qs
            .get("auth_data")
            .context("auth_data not found")?
            .to_owned();

        let body = self
            .client
            .bb_course_video_sub_info(&course_id, &sub_id, &app_id, &auth_data)
            .await?;

        Ok(body)
    }

    fn get_media_path(&self, text: &str) -> anyhow::Result<MediaPath> {
        let sub = serde_json::from_str::<SubInfo>(text).context("parse sub info failed")?;

        #[derive(Debug, serde::Deserialize)]
        struct SubInfo {
            list: Vec<SubItem>,
        }

        #[derive(Debug, serde::Deserialize)]
        struct SubItem {
            sub_content: String,
        }

        #[derive(Debug, serde::Deserialize)]
        struct SubContent {
            save_playback: SavePlayback,
        }

        #[derive(Debug, serde::Deserialize)]
        struct SavePlayback {
            is_m3u8: String,
            contents: String,
        }

        let Some(item) = sub.list.first() else {
            anyhow::bail!("sub list is empty, got {}", text);
        };

        let sub_content = serde_json::from_str::<SubContent>(&item.sub_content)
            .context("parse sub content failed")?;

        let is_m3u8 = sub_content.save_playback.is_m3u8;
        let url = sub_content.save_playback.contents;

        if is_m3u8 == "yes" {
            return Ok(MediaPath::M3u8(url));
        }

        if url.ends_with(".mp4") {
            return Ok(MediaPath::Mp4(url));
        }

        anyhow::bail!("not m3u8 or mp4, got {}", item.sub_content);
    }

    async fn get_m3u8_playlist(&self, url: &str) -> anyhow::Result<bytes::Bytes> {
        let res = self.client.get_by_uri(url).await?;
        anyhow::ensure!(res.status().is_success(), "status not success");
        let rbody = res.bytes().await?;
        Ok(rbody)
    }

    async fn _get(&self) -> anyhow::Result<(String, bytes::Bytes)> {
        let loc = self.get_iframe_url().await?;
        loop {
            let info = self.get_sub_info(&loc).await?;
            let media = self.get_media_path(&info)?;
            match media {
                MediaPath::M3u8(pl_url) => {
                    let pl_raw = self.get_m3u8_playlist(&pl_url).await?;
                    break Ok((pl_url, pl_raw));
                }
                MediaPath::Mp4(url) => {
                    log::warn!("mp4 ({url}) not supported yet, try again...");
                    compio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
        }
    }

    #[cfg(feature = "m3u8-rs")]
    pub async fn get(&self) -> anyhow::Result<CourseVideo> {
        let (pl_url, pl_raw) = self._get().await.with_context(|| {
            format!(
                "get course video for {} {}",
                self.course.title(),
                self.meta().title()
            )
        })?;

        let pl_raw = pl_raw.to_vec();
        let (_, pl) = m3u8_rs::parse_playlist(&pl_raw)
            .map_err(|e| anyhow::anyhow!("{:#}", e))
            .context("parse m3u8 failed")?;

        match pl {
            m3u8_rs::Playlist::MasterPlaylist(_) => {
                anyhow::bail!("master playlist not supported")
            }
            m3u8_rs::Playlist::MediaPlaylist(pl) => Ok(CourseVideo {
                client: self.client.clone(),
                course: self.course.clone(),
                meta: self.meta.clone(),
                pl_url: pl_url.into_url().context("parse pl_url failed")?,
                pl_raw: pl_raw.into(),
                pl,
            }),
        }
    }
}

enum MediaPath {
    M3u8(String),
    Mp4(String),
}

#[derive(Debug)]
pub struct CourseVideo {
    client: Client,
    course: Arc<CourseMeta>,
    meta: Arc<CourseVideoMeta>,
    pl_raw: bytes::Bytes,
    pl_url: url::Url,
    #[cfg(feature = "m3u8-rs")]
    pl: m3u8_rs::MediaPlaylist,
}

impl CourseVideo {
    pub fn course_name(&self) -> &str {
        self.course.name()
    }

    pub fn meta(&self) -> &CourseVideoMeta {
        &self.meta
    }

    pub fn m3u8_raw(&self) -> bytes::Bytes {
        self.pl_raw.clone()
    }

    #[cfg(feature = "m3u8-rs")]
    pub fn len_segments(&self) -> usize {
        self.pl.segments.len()
    }

    /// Refresh the key for the given segment index. You should call this method before getting the segment data referenced by the index.
    ///
    /// The EXT-X-KEY tag specifies how to decrypt them.  It applies to every Media Segment and to every Media
    /// Initialization Section declared by an EXT-X-MAP tag that appears
    /// between it and the next EXT-X-KEY tag in the Playlist file with the
    /// same KEYFORMAT attribute (or the end of the Playlist file).
    #[cfg(feature = "m3u8-rs")]
    pub fn refresh_key<'a>(
        &'a self,
        index: usize,
        key: Option<&'a m3u8_rs::Key>,
    ) -> Option<&'a m3u8_rs::Key> {
        let seg = &self.pl.segments[index];
        fn fallback_keyformat(key: &m3u8_rs::Key) -> &str {
            key.keyformat.as_deref().unwrap_or("identity")
        }

        if let Some(newkey) = &seg.key
            && key.is_none_or(|k| fallback_keyformat(k) == fallback_keyformat(newkey))
        {
            return Some(newkey);
        }
        key
    }

    #[cfg(feature = "m3u8-rs")]
    pub fn segment(&self, index: usize) -> &m3u8_rs::MediaSegment {
        &self.pl.segments[index]
    }

    /// Fetch the segment data for the given index. If `key` is provided, the segment will be decrypted.
    #[cfg(feature = "video-download")]
    pub async fn get_segment_data<'a>(
        &'a self,
        index: usize,
        key: Option<&'a m3u8_rs::Key>,
    ) -> anyhow::Result<bytes::Bytes> {
        log::info!(
            "downloading segment {}/{} for video {}",
            index,
            self.len_segments(),
            self.meta.title()
        );

        let seg = &self.pl.segments[index];

        // fetch maybe encrypted segment data
        let seg_url: String = self.pl_url.join(&seg.uri).context("join seg url")?.into();
        let mut bytes = with_cache_bytes(
            &format!("CourseVideo::download_segment_{seg_url}"),
            self.client.download_artifact_ttl(),
            self._download_segment(&seg_url),
        )
        .await
        .context("download segment data")?;

        // decrypt it if needed
        if let Some(key) = key {
            // sequence number may be used to construct IV
            let seq = (self.pl.media_sequence as usize + index) as u128;
            bytes = match self.decrypt_segment(key, bytes, seq).await {
                Ok(bytes) => bytes,
                Err(e) => {
                    log::warn!(
                        "decrypt cached segment #{index} failed, retrying without cache: {e:#}"
                    );
                    let fresh_bytes = self
                        ._download_segment(&seg_url)
                        .await
                        .context("redownload segment data")?;
                    self.decrypt_segment(key, fresh_bytes, seq)
                        .await
                        .context("decrypt redownloaded segment data")?
                }
            };
        }

        Ok(bytes)
    }

    async fn _download_segment(&self, seg_url: &str) -> anyhow::Result<bytes::Bytes> {
        let res = self.client.get_by_uri(seg_url).await?;
        anyhow::ensure!(res.status().is_success(), "status not success");

        let bytes = res.bytes().await?;
        Ok(bytes)
    }

    async fn get_aes128_key(&self, url: &str) -> anyhow::Result<[u8; 16]> {
        // fetch aes128 key from uri
        let r = with_cache_bytes(
            &format!("CourseVideo::get_aes128_uri_{url}"),
            self.client.download_artifact_ttl(),
            async {
                let r = self.client.get_by_uri(url).await?.bytes().await?;
                Ok(r)
            },
        )
        .await?
        .to_vec();

        if r.len() != 16 {
            anyhow::bail!("key length not 16: {:?}", String::from_utf8(r));
        }

        // convert to array
        let mut key = [0; 16];
        key.copy_from_slice(&r);
        Ok(key)
    }

    #[cfg(feature = "video-download")]
    async fn decrypt_segment(
        &self,
        key: &m3u8_rs::Key,
        bytes: bytes::Bytes,
        seq: u128,
    ) -> anyhow::Result<bytes::Bytes> {
        use aes::cipher::{
            BlockDecryptMut, KeyIvInit, block_padding::Pkcs7, generic_array::GenericArray,
        };
        // ref: https://datatracker.ietf.org/doc/html/rfc8216#section-4.3.2.4
        match &key.method {
            // An encryption method of AES-128 signals that Media Segments are
            // completely encrypted using [AES_128] with a 128-bit key, Cipher
            // Block Chaining, and PKCS7 padding [RFC5652].  CBC is restarted
            // on each segment boundary, using either the IV attribute value
            // or the Media Sequence Number as the IV; see Section 5.2.  The
            // URI attribute is REQUIRED for this METHOD.
            m3u8_rs::KeyMethod::AES128 => {
                let uri = key.uri.as_ref().context("key uri not found")?;
                let iv = if let Some(iv) = &key.iv {
                    let iv = iv.to_ascii_uppercase();
                    let hx = iv.strip_prefix("0x").context("iv not start with 0x")?;
                    u128::from_str_radix(hx, 16).context("parse iv failed")?
                } else {
                    seq
                }
                .to_be_bytes();

                let aes_key = self.get_aes128_key(uri).await?;

                let aes_key = GenericArray::from(aes_key);
                let iv = GenericArray::from(iv);

                let de = cbc::Decryptor::<aes::Aes128>::new(&aes_key, &iv)
                    .decrypt_padded_vec_mut::<Pkcs7>(&bytes)
                    .context("decrypt failed")?;

                Ok(de.into())
            }
            r => unimplemented!("m3u8 key: {:?}", r),
        }
    }
}
