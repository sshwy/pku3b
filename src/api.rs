use aes::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7, generic_array::GenericArray};
use anyhow::Context;
use chrono::TimeZone;
use cyper::IntoUrl;
use rand::{distr::Open01, prelude::*};
use std::{collections::HashMap, str::FromStr, sync::Arc};

use crate::{
    qs,
    utils::{with_cache, with_cache_bytes},
};

struct ClientInner {
    http_client: cyper::Client,
    cache_ttl: Option<std::time::Duration>,
    download_artifact_ttl: Option<std::time::Duration>,
}

impl std::fmt::Debug for ClientInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientInner")
            .field("cache_ttl", &self.cache_ttl)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct Client(Arc<ClientInner>);

impl std::ops::Deref for Client {
    type Target = cyper::Client;

    fn deref(&self) -> &Self::Target {
        &self.0.http_client
    }
}

impl Client {
    pub fn new(
        cache_ttl: Option<std::time::Duration>,
        download_artifact_ttl: Option<std::time::Duration>,
    ) -> Self {
        let mut default_headers = http::HeaderMap::new();
        default_headers.insert("user-agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36".parse().unwrap());
        let http_client = cyper::Client::builder()
            .cookie_store(true)
            .default_headers(default_headers)
            .build();
        Self(
            ClientInner {
                http_client,
                cache_ttl,
                download_artifact_ttl,
            }
            .into(),
        )
    }

    pub async fn blackboard(&self, username: &str, password: &str) -> anyhow::Result<Blackboard> {
        let c = &self.0.http_client;
        let mut rng = rand::rng();

        let res = c
            .post(OAUTH_LOGIN)?
            .form(&[
                ("appid", "blackboard"),
                ("userName", username),
                ("password", password),
                ("randCode", ""),
                ("smsCode", ""),
                ("otpCode", ""),
                ("redirUrl", REDIR_URL),
            ])?
            .send()
            .await?;

        // dbg!(res.headers());
        let rbody = res.text().await?;
        let value = serde_json::Value::from_str(&rbody)?;
        let token = value.as_object().context("resp not an object")?["token"]
            .as_str()
            .context("property 'token' not found on object")?
            .to_owned();

        if std::env::var("PKU3B_DEBUG").is_ok() {
            eprintln!("iaaa oauth token: {token}");
        }

        let _rand: f64 = rng.sample(Open01);
        let _rand = &format!("{_rand:.20}");

        let res = c
            .get(SSO_LOGIN)?
            .query(&[("_rand", _rand), ("token", &token)])?
            .send()
            .await?;

        anyhow::ensure!(res.status().is_success(), "status not success");
        // dbg!(res.headers());

        Ok(Blackboard {
            client: self.clone(),
        })
    }

    pub fn cache_ttl(&self) -> Option<&std::time::Duration> {
        self.0.cache_ttl.as_ref()
    }

    pub fn download_artifact_ttl(&self) -> Option<&std::time::Duration> {
        self.0.download_artifact_ttl.as_ref()
    }
}

#[derive(Debug)]
pub struct Blackboard {
    client: Client,
    // token: String,
}

const OAUTH_LOGIN: &str = "https://iaaa.pku.edu.cn/iaaa/oauthlogin.do";
const REDIR_URL: &str =
    "http://course.pku.edu.cn/webapps/bb-sso-BBLEARN/execute/authValidate/campusLogin";
const SSO_LOGIN: &str =
    "https://course.pku.edu.cn/webapps/bb-sso-BBLEARN/execute/authValidate/campusLogin";
const BLACKBOARD_HOME_PAGE: &str =
    "https://course.pku.edu.cn/webapps/portal/execute/tabs/tabAction";
const COURSE_INFO_PAGE: &str = "https://course.pku.edu.cn/webapps/blackboard/execute/announcement";

impl Blackboard {
    async fn _get_courses(&self) -> anyhow::Result<Vec<(String, String)>> {
        let res = self
            .client
            .get(BLACKBOARD_HOME_PAGE)?
            .query(&[("tab_tab_group_id", "_1_1")])?
            .send()
            .await?;

        anyhow::ensure!(res.status().is_success(), "status not success");
        let rbody = res.text().await?;
        let dom = scraper::Html::parse_document(&rbody);
        // the first one contains the courses in the current semester
        let ul = dom
            .select(&scraper::Selector::parse("ul.courseListing").unwrap())
            .nth(0)
            .context("courses not found")?;

        let re = regex::Regex::new(r"key=([\d_]+),").unwrap();
        let courses = ul
            .select(&scraper::Selector::parse("li a").unwrap())
            .map(|a| {
                let href = a.value().attr("href").unwrap();
                let text = a.text().collect::<String>();
                // use regex to extract course key (of form key=_80052_1)

                let key = re
                    .captures(href)
                    .context("course key not found")?
                    .get(1)
                    .context("course key not found")?
                    .as_str()
                    .to_owned();

                Ok((key, text))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        Ok(courses)
    }
    pub async fn get_courses(&self) -> anyhow::Result<Vec<CourseHandle>> {
        let courses =
            with_cache("_get_courses", self.client.cache_ttl(), self._get_courses()).await?;

        if std::env::var("PKU3B_DEBUG").is_ok() {
            dbg!(&courses);
        }

        let courses = courses
            .into_iter()
            .map(|(key, title)| {
                Ok(CourseHandle {
                    client: self.client.clone(),
                    meta: CourseMeta { key, title }.into(),
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        Ok(courses)
    }
}

#[derive(Debug)]
struct CourseMeta {
    key: String,
    title: String,
}

#[derive(Debug, Clone)]
pub struct CourseHandle {
    client: Client,
    meta: Arc<CourseMeta>,
}

impl CourseHandle {
    pub async fn _get(&self) -> anyhow::Result<HashMap<String, String>> {
        let res = self
            .client
            .get(COURSE_INFO_PAGE)?
            .query(&[
                ("method", "search"),
                ("context", "course_entry"),
                ("course_id", &self.meta.key),
                ("handle", "announcements_entry"),
                ("mode", "view"),
            ])?
            .send()
            .await?;

        anyhow::ensure!(res.status().is_success(), "status not success");
        let rbody = res.text().await?;
        let dom = scraper::Html::parse_document(&rbody);
        let entries = dom
            .select(&scraper::Selector::parse("#courseMenuPalette_contents > li > a").unwrap())
            .map(|a| {
                let text = a.text().collect::<String>();
                let href = a.value().attr("href").unwrap();
                Ok((text, href.to_owned()))
            })
            .collect::<anyhow::Result<HashMap<_, _>>>()?;

        Ok(entries)
    }

    pub async fn get(&self) -> anyhow::Result<Course> {
        let entries = with_cache(
            &format!("CourseHandle::_get_{}", self.meta.key),
            self.client.cache_ttl(),
            self._get(),
        )
        .await?;

        Ok(Course {
            client: self.client.clone(),
            meta: self.meta.clone(),
            entries,
        })
    }
}

#[derive(Debug)]
pub struct Course {
    client: Client,
    meta: Arc<CourseMeta>,
    entries: HashMap<String, String>,
}

impl Course {
    pub fn name(&self) -> &str {
        &self.meta.title.split(": ").nth(1).unwrap()
    }
    async fn _get_assignments(&self) -> anyhow::Result<Vec<(String, String)>> {
        let Some(uri) = self.entries.get("课程作业") else {
            return Ok(Vec::new());
        };

        let res = self
            .client
            .get(format!("https://course.pku.edu.cn{}", uri))
            .context("create request failed")?
            .send()
            .await?;
        anyhow::ensure!(res.status().is_success(), "status not success");
        let rbody = res.text().await?;
        let dom = scraper::Html::parse_document(&rbody);
        let assignments = dom
            .select(&scraper::Selector::parse("#content_listContainer > li").unwrap())
            .map(|li| {
                let title_a = li
                    .select(&scraper::Selector::parse("h3 > a").unwrap())
                    .next()
                    .context("assignment title not found")?;
                let title = title_a.text().collect::<String>();
                let href = title_a
                    .value()
                    .attr("href")
                    .context("assignment href not found")?
                    .to_owned();

                Ok((title, href))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        Ok(assignments)
    }
    pub async fn get_assignments(&self) -> anyhow::Result<Vec<CourseAssignmentsHandle>> {
        let assignments = with_cache(
            &format!("Course::_get_assignments_{}", self.meta.key),
            self.client.cache_ttl(),
            self._get_assignments(),
        )
        .await?;

        if std::env::var("PKU3B_DEBUG").is_ok() {
            eprintln!("key: {}", self.meta.key);
            dbg!(&assignments);
        }

        let assignments = assignments
            .into_iter()
            // remove assignment answers
            .filter(|(_, url)| !url.starts_with("/bbcswebdav"))
            .map(|(title, uri)| {
                let qs = qs::Query::from_str(&uri).context("parse uri qs failed")?;
                let course_id = qs
                    .get("course_id")
                    .context("course_id not found")?
                    .to_owned();
                let content_id = qs
                    .get("content_id")
                    .context("content_id not found")?
                    .to_owned();

                Ok(CourseAssignmentsHandle {
                    client: self.client.clone(),
                    meta: CourseAssignmentsMeta {
                        title,
                        course_id,
                        content_id,
                    }
                    .into(),
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        Ok(assignments)
    }
    pub fn entries(&self) -> &HashMap<String, String> {
        &self.entries
    }
    pub async fn query_launch_link(&self, uri: &str) -> anyhow::Result<String> {
        let res = self
            .client
            .get(format!("https://course.pku.edu.cn{}", uri))?
            .send()
            .await?;

        let st = res.status();
        anyhow::ensure!(st.as_u16() == 302, "invalid status: {}", st);
        let loc = res
            .headers()
            .get("location")
            .context("location header not found")?
            .to_str()
            .context("location header not str")?
            .to_owned();

        Ok(loc)
    }
    pub async fn get_video_list(&self) -> anyhow::Result<Vec<CourseVideoHandle>> {
        let videos = with_cache(
            &format!("Course::get_video_list_{}", self.meta.key),
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
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        Ok(videos)
    }
    async fn _get_video_list(&self) -> anyhow::Result<Vec<CourseVideoMeta>> {
        let Some(uri) = self.entries().get("课堂实录") else {
            anyhow::bail!("课堂实录 entry not found");
        };

        let uri = self.query_launch_link(uri).await?;
        let url = format!("https://course.pku.edu.cn{}", uri);

        let res = self.client.get(&url)?.send().await?;

        let u = url.into_url()?;

        let rbody = res.text().await?;
        let dom = scraper::Html::parse_document(&rbody);
        let videos = dom
            .select(&scraper::Selector::parse("tbody#listContainer_databody > tr").unwrap())
            .map(|tr| {
                let title = tr
                    .child_elements()
                    .nth(0)
                    .unwrap()
                    .text()
                    .collect::<String>();
                let s = scraper::Selector::parse("span.table-data-cell-value").unwrap();
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

#[derive(Debug)]
pub struct CourseAssignmentsMeta {
    title: String,
    course_id: String,
    content_id: String,
}

#[derive(Debug, Clone)]
pub struct CourseAssignmentsHandle {
    client: Client,
    meta: Arc<CourseAssignmentsMeta>,
}

impl CourseAssignmentsHandle {
    async fn _get(&self) -> anyhow::Result<CourseAssignmentData> {
        let res = self
            .client
            .get("https://course.pku.edu.cn/webapps/assignment/uploadAssignment")?
            .query(&[
                ("action", "newAttempt"),
                ("content_id", &self.meta.content_id),
                ("course_id", &self.meta.course_id),
            ])?
            .send()
            .await?;
        anyhow::ensure!(res.status().is_success(), "status not success");
        let rbody = res.text().await?;
        let dom = scraper::Html::parse_document(&rbody);

        let desc = if let Some(el) = dom
            .select(&scraper::Selector::parse("#instructions div.vtbegenerated").unwrap())
            .next()
        {
            el.child_elements()
                .map(|p| p.text().collect::<String>().trim().to_owned())
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        let attachments = dom
            .select(&scraper::Selector::parse("#instructions div.field > a").unwrap())
            .map(|a| {
                let text = a.text().collect::<String>();
                let href = a.value().attr("href").unwrap();
                let text = if let Some(text) = text.strip_prefix("\u{a0}") {
                    text.to_owned()
                } else {
                    text
                };
                Ok((text, href.to_owned()))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        let deadline = dom
            .select(&scraper::Selector::parse("#assignMeta2 + div").unwrap())
            .next()
            .context("deadline el not found")?
            .text()
            .collect::<String>();

        // replace consecutive whitespaces with a single space
        let deadline = deadline.split_whitespace().collect::<Vec<_>>().join(" ");

        let attempt = self._get_current_attempt().await?;

        Ok(CourseAssignmentData {
            descriptions: desc,
            attachments,
            deadline,
            attempt,
        })
    }
    pub async fn get(&self) -> anyhow::Result<CourseAssignments> {
        let data = with_cache(
            &format!(
                "CourseAssignmentsHandle::_get_{}_{}",
                self.meta.content_id, self.meta.course_id
            ),
            self.client.cache_ttl(),
            self._get(),
        )
        .await?;

        Ok(CourseAssignments {
            _client: self.client.clone(),
            meta: self.meta.clone(),
            data,
        })
    }

    async fn _get_current_attempt(&self) -> anyhow::Result<Option<String>> {
        let res = self
            .client
            .get("https://course.pku.edu.cn/webapps/assignment/uploadAssignment")?
            .query(&[
                ("mode", "view"),
                ("content_id", &self.meta.content_id),
                ("course_id", &self.meta.course_id),
            ])?
            .send()
            .await?;
        anyhow::ensure!(res.status().is_success(), "status not success");

        let rbody = res.text().await?;
        let dom = scraper::Html::parse_document(&rbody);
        let attempt_label = if let Some(e) = dom
            .select(&scraper::Selector::parse("h3#currentAttempt_label").unwrap())
            .next()
        {
            e.text().collect::<String>()
        } else {
            return Ok(None);
        };

        let attempt_label = attempt_label
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");

        Ok(Some(attempt_label))
    }
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct CourseAssignmentData {
    descriptions: Vec<String>,
    attachments: Vec<(String, String)>,
    deadline: String,
    attempt: Option<String>,
}

pub struct CourseAssignments {
    _client: Client,
    meta: Arc<CourseAssignmentsMeta>,
    data: CourseAssignmentData,
}

impl CourseAssignments {
    pub fn title(&self) -> &str {
        &self.meta.title
    }

    pub fn descriptions(&self) -> &[String] {
        &self.data.descriptions
    }

    pub fn attachments(&self) -> &[(String, String)] {
        &self.data.attachments
    }

    pub fn last_attempt(&self) -> Option<&str> {
        self.data.attempt.as_deref()
    }

    /// Try to parse the deadline string into a NaiveDateTime.
    pub fn deadline(&self) -> Option<chrono::DateTime<chrono::Local>> {
        let re = regex::Regex::new(
            r"(\d{4})年(\d{1,2})月(\d{1,2})日 星期. (上午|下午)(\d{1,2}):(\d{1,2})",
        )
        .unwrap();

        if let Some(caps) = re.captures(&self.data.deadline) {
            let year: i32 = caps[1].parse().ok()?;
            let month: u32 = caps[2].parse().ok()?;
            let day: u32 = caps[3].parse().ok()?;
            let mut hour: u32 = caps[5].parse().ok()?;
            let minute: u32 = caps[6].parse().ok()?;

            // Adjust for PM times
            if &caps[4] == "下午" && hour < 12 {
                hour += 12;
            }

            // Create NaiveDateTime
            let naive_dt = chrono::NaiveDateTime::new(
                chrono::NaiveDate::from_ymd_opt(year, month, day)?,
                chrono::NaiveTime::from_hms_opt(hour, minute, 0)?,
            );

            let r = chrono::Local.from_local_datetime(&naive_dt).unwrap();

            Some(r)
        } else {
            None
        }
    }

    pub fn deadline_raw(&self) -> &str {
        &self.data.deadline
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct CourseVideoMeta {
    title: String,
    time: String,
    url: String,
}

#[derive(Debug)]
pub struct CourseVideoHandle {
    client: Client,
    meta: Arc<CourseVideoMeta>,
}

impl CourseVideoHandle {
    async fn get_iframe_url(&self) -> anyhow::Result<String> {
        let res = self.client.get(&self.meta.url)?.send().await?;
        anyhow::ensure!(res.status().is_success(), "status not success");
        let rbody = res.text().await?;
        let dom = scraper::Html::parse_document(&rbody);
        let iframe = dom
            .select(&scraper::Selector::parse("#content iframe").unwrap())
            .next()
            .context("iframe not found")?;
        let src = iframe
            .value()
            .attr("src")
            .context("src not found")?
            .to_owned();

        let res = self.client.get(&src)?.send().await?;
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

    async fn get_sub_info(&self, loc: &str) -> anyhow::Result<serde_json::Value> {
        let qs = qs::Query::from_str(&loc).context("parse loc qs failed")?;
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

        let res = self
            .client
            .get("https://yjapise.pku.edu.cn/courseapi/v2/schedule/get-sub-info-by-auth-data")?
            .query(&[
                ("all", "1"),
                ("course_id", &course_id),
                ("sub_id", &sub_id),
                ("with_sub_data", "1"),
                ("app_id", &app_id),
                ("auth_data", &auth_data),
            ])?
            .send()
            .await?;

        anyhow::ensure!(res.status().is_success(), "status not success");
        let rbody = res.text().await?;
        let value = serde_json::Value::from_str(&rbody)?;

        Ok(value)
    }

    fn get_m3u8_path(&self, sub_info: serde_json::Value) -> anyhow::Result<String> {
        let sub_content = sub_info
            .as_object()
            .context("sub_info not object")?
            .get("list")
            .context("sub_info.list not found")?
            .as_array()
            .context("sub_info.list not array")?
            .get(0)
            .context("sub_info.list empty")?
            .as_object()
            .context("sub_info.list[0] not object")?
            .get("sub_content")
            .context("sub_info.list[0].sub_content not found")?
            .as_str()
            .context("sub_info.list[0].sub_content not string")?;

        let sub_content = serde_json::Value::from_str(sub_content)?;

        let save_playback = sub_content
            .as_object()
            .context("sub_content not object")?
            .get("save_playback")
            .context("sub_content.save_playback not found")?
            .as_object()
            .context("sub_content.save_playback not object")?;

        let is_m3u8 = save_playback
            .get("is_m3u8")
            .context("sub_content.save_playback.is_m3u8 not found")?
            .as_str()
            .context("sub_content.save_playback.is_m3u8 not string")?;

        anyhow::ensure!(is_m3u8 == "yes", "not m3u8");

        let url = save_playback
            .get("contents")
            .context("save_playback.contents not found")?
            .as_str()
            .context("save_playback.contents not string")?;

        Ok(url.to_owned())
    }

    async fn get_m3u8_playlist(&self, url: &str) -> anyhow::Result<String> {
        let res = self.client.get(url)?.send().await?;
        anyhow::ensure!(res.status().is_success(), "status not success");
        let rbody = res.text().await?;
        Ok(rbody)
    }

    async fn _get(&self) -> anyhow::Result<(String, String)> {
        let loc = self.get_iframe_url().await?;
        let info = self.get_sub_info(&loc).await?;
        let pl_url = self.get_m3u8_path(info)?;
        let pl_raw = self.get_m3u8_playlist(&pl_url).await?;
        Ok((pl_url, pl_raw))
    }

    pub async fn get(&self) -> anyhow::Result<CourseVideo> {
        let (pl_url, pl_raw) = with_cache(
            &format!("CourseVideoHandle::_get_{}", self.meta.url),
            self.client.cache_ttl(),
            self._get(),
        )
        .await?;

        let (_, pl) = m3u8_rs::parse_playlist(pl_raw.as_bytes())
            .map_err(|e| anyhow::anyhow!("{:#}", e))
            .context("parse m3u8 failed")?;

        match pl {
            m3u8_rs::Playlist::MasterPlaylist(_) => anyhow::bail!("master playlist not supported"),
            m3u8_rs::Playlist::MediaPlaylist(pl) => Ok(CourseVideo {
                client: self.client.clone(),
                meta: self.meta.clone(),
                pl_url: pl_url.into_url().context("parse pl_url failed")?,
                pl,
            }),
        }
    }
}

#[derive(Debug)]
pub struct CourseVideo {
    client: Client,
    meta: Arc<CourseVideoMeta>,
    pl_url: url::Url,
    pl: m3u8_rs::MediaPlaylist,
}

impl CourseVideo {
    pub async fn download_segment(&self, index: usize) -> anyhow::Result<bytes::Bytes> {
        let seg = &self.pl.segments[0];

        let seg_url = self.pl_url.join(&seg.uri).context("join seg url").unwrap();
        let seg_url = seg_url.as_str();

        // get maybe encrypted segment data
        let mut bytes = with_cache_bytes(
            &format!("CourseVideo::download_segment_{}", seg_url),
            self.client.download_artifact_ttl(),
            self._download_segment(seg_url),
        )
        .await?;

        // decrypt if needed
        let seq = (self.pl.media_sequence as usize + index) as u128;
        if let Some(key) = &seg.key {
            bytes = self.decode_segment(key, bytes, seq).await?;
        }

        Ok(bytes)
    }

    async fn _download_segment(&self, seg_url: &str) -> anyhow::Result<bytes::Bytes> {
        let res = self.client.get(seg_url)?.send().await?;
        anyhow::ensure!(res.status().is_success(), "status not success");

        let bytes = res.bytes().await?;
        Ok(bytes)
    }

    async fn get_aes128_key(&self, uri: &str) -> anyhow::Result<[u8; 16]> {
        // fetch aes128 key from uri
        let r = with_cache_bytes(
            &format!("CourseVideo::get_aes128_uri"),
            self.client.download_artifact_ttl(),
            async {
                let r = self.client.get(uri)?.send().await?.bytes().await?;
                Ok(r)
            },
        )
        .await?
        .to_vec();

        if r.len() != 16 {
            anyhow::bail!("key length not 16: {:?}", r);
        }

        // convert to array
        let mut key = [0; 16];
        key.copy_from_slice(&r);
        Ok(key)
    }

    async fn decode_segment(
        &self,
        key: &m3u8_rs::Key,
        bytes: bytes::Bytes,
        seq: u128,
    ) -> anyhow::Result<bytes::Bytes> {
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
                    .decrypt_padded_vec_mut::<Pkcs7>(&mut bytes.to_vec())
                    .context("decrypt failed")?;

                Ok(de.into())
            }
            r => unimplemented!("m3u8 key: {:?}", r),
        }
    }
}
