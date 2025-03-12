mod low_level;

use anyhow::Context;
use chrono::TimeZone;
use cyper::IntoUrl;
use std::{
    collections::HashMap,
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
const AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36";

struct ClientInner {
    http_client: low_level::LowLevelClient,
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
        let mut default_headers = http::HeaderMap::new();
        default_headers.insert(http::header::USER_AGENT, AGENT.parse().unwrap());
        let http_client = cyper::Client::builder()
            .cookie_store(true)
            .default_headers(default_headers)
            .build();

        log::info!("Cache TTL: {:?}", cache_ttl);
        log::info!("Download Artifact TTL: {:?}", download_artifact_ttl);

        Self(
            ClientInner {
                http_client: low_level::LowLevelClient::from_cyper_client(http_client),
                cache_ttl,
                download_artifact_ttl,
            }
            .into(),
        )
    }

    pub fn new_nocache() -> Self {
        Self::new(None, None)
    }

    pub async fn blackboard(&self, username: &str, password: &str) -> anyhow::Result<Blackboard> {
        let c = &self.0.http_client;
        let value = c.oauth_login(username, password).await?;
        let token = value
            .as_object()
            .context("value not an object")?
            .get("token")
            .context("password not correct")?
            .as_str()
            .context("property 'token' not string")?
            .to_owned();
        c.blackboard_sso_login(&token).await?;

        log::debug!("iaaa oauth token for {username}: {token}");

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

impl Default for Client {
    fn default() -> Self {
        Self::new(Some(ONE_HOUR), Some(ONE_DAY))
    }
}

#[derive(Debug)]
pub struct Blackboard {
    client: Client,
    // token: String,
}

impl Blackboard {
    async fn _get_courses(&self) -> anyhow::Result<Vec<(String, String)>> {
        let dom = self.client.blackboard_homepage().await?;

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
        log::info!("fetching courses...");

        let courses = with_cache(
            "Blackboard::_get_courses",
            self.client.cache_ttl(),
            self._get_courses(),
        )
        .await?;

        let courses = courses
            .into_iter()
            .map(|(key, long_title)| {
                Ok(CourseHandle {
                    client: self.client.clone(),
                    meta: CourseMeta { key, long_title }.into(),
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        Ok(courses)
    }
}

#[derive(Debug)]
pub struct CourseMeta {
    key: String,
    long_title: String,
}

impl CourseMeta {
    /// Course Name (semester)
    pub fn title(&self) -> &str {
        self.long_title.split_once(":").unwrap().1.trim()
    }

    /// Cousre Name
    pub fn name(&self) -> &str {
        let s = self.title();
        let i = s
            .char_indices()
            .filter(|(_, c)| *c == '(')
            .last()
            .unwrap()
            .0;
        s.split_at(i).0.trim()
    }
}

#[derive(Debug, Clone)]
pub struct CourseHandle {
    client: Client,
    meta: Arc<CourseMeta>,
}

impl CourseHandle {
    pub async fn _get(&self) -> anyhow::Result<HashMap<String, String>> {
        let dom = self.client.blackboard_coursepage(&self.meta.key).await?;

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
        log::info!("fetching course {}", self.meta.title());

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
    pub fn meta(&self) -> &CourseMeta {
        &self.meta
    }
    async fn _get_assignments(&self) -> anyhow::Result<Vec<(String, String)>> {
        let Some(uri) = self.entries.get("课程作业") else {
            log::warn!("课程作业 entry not found for course {}", self.meta.title());
            return Ok(Vec::new());
        };

        let dom = self
            .client
            .page_by_uri(uri)
            .await
            .context("get course assignments page")?;

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
    pub async fn get_assignments(&self) -> anyhow::Result<Vec<CourseAssignmentHandle>> {
        log::info!(
            "fetching assignments for {} (key={})",
            self.meta.title(),
            self.meta.key
        );

        let assignments = with_cache(
            &format!("Course::_get_assignments_{}", self.meta.key),
            self.client.cache_ttl(),
            self._get_assignments(),
        )
        .await?;

        log::debug!("assignments: {:?}", assignments);

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

                Ok(CourseAssignmentHandle {
                    client: self.client.clone(),
                    course: self.meta.clone(),
                    meta: CourseAssignmentMeta {
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
        let res = self.client.get_by_uri(uri).await?;
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
        log::info!("fetching video list for course {}", self.meta.title());

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
                    course: self.meta.clone(),
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        Ok(videos)
    }
    async fn _get_video_list(&self) -> anyhow::Result<Vec<CourseVideoMeta>> {
        let Some(uri) = self.entries().get("课堂实录") else {
            log::warn!("课堂实录 entry not found for course {}", self.meta.title());
            return Ok(vec![]);
        };

        let uri = self.query_launch_link(uri).await?;
        let url = format!("https://course.pku.edu.cn{}", uri);
        let u = url.into_url()?;

        let dom = self.client.page_by_uri(&uri).await?;

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
pub struct CourseAssignmentMeta {
    title: String,
    course_id: String,
    content_id: String,
}

#[derive(Debug, Clone)]
pub struct CourseAssignmentHandle {
    client: Client,
    course: Arc<CourseMeta>,
    meta: Arc<CourseAssignmentMeta>,
}

impl CourseAssignmentHandle {
    pub fn id(&self) -> String {
        let mut hasher = std::hash::DefaultHasher::new();
        self.course.key.hash(&mut hasher);
        self.meta.content_id.hash(&mut hasher);
        self.meta.course_id.hash(&mut hasher);
        let x = hasher.finish();
        format!("{x:x}")
    }

    async fn _get(&self) -> anyhow::Result<CourseAssignmentData> {
        let dom = self
            .client
            .blackboard_course_assignment_uploadpage(&self.meta.course_id, &self.meta.content_id)
            .await?;

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
            .map(|e| {
                // replace consecutive whitespaces with a single space
                e.text()
                    .collect::<String>()
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            });

        let attempt = self._get_current_attempt().await?;

        Ok(CourseAssignmentData {
            descriptions: desc,
            attachments,
            deadline,
            attempt,
        })
    }
    pub async fn get(&self) -> anyhow::Result<CourseAssignment> {
        let data = with_cache(
            &format!(
                "CourseAssignmentHandle::_get_{}_{}",
                self.meta.content_id, self.meta.course_id
            ),
            self.client.cache_ttl(),
            self._get(),
        )
        .await?;

        Ok(CourseAssignment {
            client: self.client.clone(),
            meta: self.meta.clone(),
            data,
        })
    }

    async fn _get_current_attempt(&self) -> anyhow::Result<Option<String>> {
        let dom = self
            .client
            .blackboard_course_assignment_viewpage(&self.meta.course_id, &self.meta.content_id)
            .await?;

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
    deadline: Option<String>,
    attempt: Option<String>,
}

pub struct CourseAssignment {
    client: Client,
    meta: Arc<CourseAssignmentMeta>,
    data: CourseAssignmentData,
}

impl CourseAssignment {
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

    pub async fn get_submit_formfields(&self) -> anyhow::Result<HashMap<String, String>> {
        let dom = self
            .client
            .blackboard_course_assignment_uploadpage(&self.meta.course_id, &self.meta.content_id)
            .await?;

        let extract_field = |input: scraper::ElementRef<'_>| {
            let name = input.value().attr("name")?.to_owned();
            let value = input.value().attr("value")?.to_owned();
            Some((name, value))
        };

        let submitformfields = dom
            .select(&scraper::Selector::parse("form#uploadAssignmentFormId input").unwrap())
            .map(extract_field)
            .chain(
                dom.select(&scraper::Selector::parse("div.field input").unwrap())
                    .map(extract_field),
            )
            .flatten()
            .collect::<HashMap<_, _>>();

        Ok(submitformfields)
    }

    pub async fn submit_file(&self, path: &std::path::Path) -> anyhow::Result<()> {
        log::info!("submitting file: {}", path.display());

        let ext = path
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let content_type = get_mime_type(&ext);
        log::info!("content type: {}", content_type);

        let filename = path
            .file_name()
            .context("file name not found")?
            .to_string_lossy()
            .to_string();

        let map = self.get_submit_formfields().await?;
        log::trace!("map: {:#?}", map);

        macro_rules! add_field_from_map {
            ($body:ident, $name:expr) => {
                let $body = $body.add_field(
                    $name,
                    map.get($name)
                        .with_context(|| format!("field '{}' not found", $name))?
                        .as_bytes(),
                );
            };
        }

        let body = multipart::MultipartBuilder::new();
        add_field_from_map!(body, "attempt_id");
        add_field_from_map!(body, "blackboard.platform.security.NonceUtil.nonce");
        add_field_from_map!(body, "blackboard.platform.security.NonceUtil.nonce.ajax");
        add_field_from_map!(body, "content_id");
        add_field_from_map!(body, "course_id");
        add_field_from_map!(body, "isAjaxSubmit");
        add_field_from_map!(body, "lu_link_id");
        add_field_from_map!(body, "mode");
        add_field_from_map!(body, "recallUrl");
        add_field_from_map!(body, "remove_file_id");
        add_field_from_map!(body, "studentSubmission.text_f");
        add_field_from_map!(body, "studentSubmission.text_w");
        add_field_from_map!(body, "studentSubmission.type");
        add_field_from_map!(body, "student_commentstext_f");
        add_field_from_map!(body, "student_commentstext_w");
        add_field_from_map!(body, "student_commentstype");
        add_field_from_map!(body, "textbox_prefix");
        let body = body
            .add_field("studentSubmission.text", b"")
            .add_field("student_commentstext", b"")
            .add_field("dispatch", b"submit")
            .add_field("newFile_artifactFileId", b"undefined")
            .add_field("newFile_artifactType", b"undefined")
            .add_field("newFile_artifactTypeResourceKey", b"undefined")
            .add_field("newFile_attachmentType", b"L") // not sure
            .add_field("newFile_fileId", b"new")
            .add_field("newFile_linkTitle", filename.as_bytes())
            .add_field("newFilefilePickerLastInput", b"dummyValue")
            .add_file(
                "newFile_LocalFile0",
                &filename,
                content_type,
                std::fs::File::open(path)?,
            )
            .add_field("useless", b"");

        let res = self
            .client
            .blackboard_course_assignment_uploaddata(body)
            .await?;

        if !res.status().is_success() {
            let st = res.status();
            let rbody = res.text().await?;
            if rbody.contains("尝试呈现错误页面时发生严重的内部错误") {
                anyhow::bail!("invalid status {} (caused by unknown server error)", st);
            }

            log::debug!("response: {}", rbody);
            anyhow::bail!("invalid status {}", st);
        }

        Ok(())
    }

    /// Try to parse the deadline string into a NaiveDateTime.
    pub fn deadline(&self) -> Option<chrono::DateTime<chrono::Local>> {
        let d = self.data.deadline.as_deref()?;
        let re = regex::Regex::new(
            r"(\d{4})年(\d{1,2})月(\d{1,2})日 星期. (上午|下午)(\d{1,2}):(\d{1,2})",
        )
        .unwrap();

        if let Some(caps) = re.captures(d) {
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

    pub fn deadline_raw(&self) -> Option<&str> {
        self.data.deadline.as_deref()
    }

    pub async fn download_attachment(
        &self,
        uri: &str,
        dest: &std::path::Path,
    ) -> anyhow::Result<()> {
        log::debug!(
            "downloading attachment from https://course.pku.edu.cn{}",
            uri
        );
        let res = self.client.get_by_uri(uri).await?;
        anyhow::ensure!(
            res.status().as_u16() == 302,
            "status not 302: {}",
            res.status()
        );

        let loc = res
            .headers()
            .get("location")
            .context("location header not found")?
            .to_str()
            .context("location header not str")?
            .to_owned();

        log::debug!("redicted to https://course.pku.edu.cn{}", loc);
        let res = self.client.get_by_uri(&loc).await?;
        anyhow::ensure!(res.status().is_success(), "status not success");

        let rbody = res.bytes().await?;
        let r = compio::fs::write(dest, rbody).await;
        compio::buf::buf_try!(@try r);
        Ok(())
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
        self.course.key.hash(&mut hasher);
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
            .select(&scraper::Selector::parse("#content iframe").unwrap())
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

    async fn get_sub_info(&self, loc: &str) -> anyhow::Result<serde_json::Value> {
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

        let value = self
            .client
            .blackboard_course_video_sub_info(&course_id, &sub_id, &app_id, &auth_data)
            .await?;

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
            .first()
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

    async fn get_m3u8_playlist(&self, url: &str) -> anyhow::Result<bytes::Bytes> {
        let res = self.client.get_by_uri(url).await?;
        anyhow::ensure!(res.status().is_success(), "status not success");
        let rbody = res.bytes().await?;
        Ok(rbody)
    }

    async fn _get(&self) -> anyhow::Result<(String, bytes::Bytes)> {
        let loc = self.get_iframe_url().await?;
        let info = self.get_sub_info(&loc).await?;
        let pl_url = self.get_m3u8_path(info)?;
        let pl_raw = self.get_m3u8_playlist(&pl_url).await?;
        Ok((pl_url, pl_raw))
    }

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
            m3u8_rs::Playlist::MasterPlaylist(_) => anyhow::bail!("master playlist not supported"),
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

#[derive(Debug)]
pub struct CourseVideo {
    client: Client,
    course: Arc<CourseMeta>,
    meta: Arc<CourseVideoMeta>,
    pl_raw: bytes::Bytes,
    pl_url: url::Url,
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

    pub fn len_segments(&self) -> usize {
        self.pl.segments.len()
    }

    /// Refresh the key for the given segment index. You should call this method before getting the segment data referenced by the index.
    ///
    /// The EXT-X-KEY tag specifies how to decrypt them.  It applies to every Media Segment and to every Media
    /// Initialization Section declared by an EXT-X-MAP tag that appears
    /// between it and the next EXT-X-KEY tag in the Playlist file with the
    /// same KEYFORMAT attribute (or the end of the Playlist file).
    pub fn refresh_key<'a>(
        &'a self,
        index: usize,
        key: Option<&'a m3u8_rs::Key>,
    ) -> Option<&'a m3u8_rs::Key> {
        let seg = &self.pl.segments[index];
        fn fallback_keyformat(key: &m3u8_rs::Key) -> &str {
            key.keyformat.as_deref().unwrap_or("identity")
        }

        if let Some(newkey) = &seg.key {
            if key.is_none_or(|k| fallback_keyformat(k) == fallback_keyformat(newkey)) {
                return Some(newkey);
            }
        }
        key
    }

    pub fn segment(&self, index: usize) -> &m3u8_rs::MediaSegment {
        &self.pl.segments[index]
    }

    /// Fetch the segment data for the given index. If `key` is provided, the segment will be decrypted.
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
            &format!("CourseVideo::download_segment_{}", seg_url),
            self.client.download_artifact_ttl(),
            self._download_segment(&seg_url),
        )
        .await
        .context("download segment data")?;

        // decrypt it if needed
        if let Some(key) = key {
            // sequence number may be used to construct IV
            let seq = (self.pl.media_sequence as usize + index) as u128;
            bytes = self
                .decrypt_segment(key, bytes, seq)
                .await
                .context("decrypt segment data")?;
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
            &format!("CourseVideo::get_aes128_uri_{}", url),
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
