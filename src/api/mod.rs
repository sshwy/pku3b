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
        log::info!("Cache TTL: {:?}", cache_ttl);
        log::info!("Download Artifact TTL: {:?}", download_artifact_ttl);

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

    pub async fn blackboard(&self, username: &str, password: &str) -> anyhow::Result<Blackboard> {
        let c = &self.0.http_client;
        c.bb_login(username, password).await?;

        Ok(Blackboard {
            client: self.clone(),
        })
    }

    pub async fn syllabus(&self, username: &str, password: &str) -> anyhow::Result<Syllabus> {
        let c = &self.0.http_client;
        c.sb_login(username, password).await?;

        Ok(Syllabus {
            client: self.clone(),
            username: username.to_owned(),
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
    async fn _get_courses(&self) -> anyhow::Result<Vec<(String, String, bool)>> {
        let dom = self.client.bb_homepage().await?;
        let re = regex::Regex::new(r"key=([\d_]+),").unwrap();
        let ul_sel = Selector::parse("ul.courseListing").unwrap();
        let sel = Selector::parse("li a").unwrap();

        let f = |a: scraper::ElementRef<'_>| {
            let href = a.value().attr("href").unwrap();
            let text = a.text().collect::<String>();
            // use regex to extract course key (of form key=_80052_1)

            let key = re
                .captures(href)
                .and_then(|s| s.get(1))
                .context("course key not found")?
                .as_str()
                .to_owned();

            Ok((key, text))
        };

        // the first one contains the courses in the current semester
        let ul = dom.select(&ul_sel).nth(0).context("courses not found")?;
        let courses = ul.select(&sel).map(f).collect::<anyhow::Result<Vec<_>>>()?;

        // the second one contains the courses in the previous semester
        let ul_history = dom.select(&ul_sel).nth(1).context("courses not found")?;
        let courses_history = ul_history
            .select(&sel)
            .map(f)
            .collect::<anyhow::Result<Vec<_>>>()?;

        Ok(courses
            .into_iter()
            .map(|(k, t)| (k, t, true))
            .chain(courses_history.into_iter().map(|(k, t)| (k, t, false)))
            .collect())
    }
    pub async fn get_courses(&self, only_current: bool) -> anyhow::Result<Vec<CourseHandle>> {
        log::info!("fetching courses...");

        let courses = with_cache(
            "Blackboard::_get_courses",
            self.client.cache_ttl(),
            self._get_courses(),
        )
        .await?;

        let mut courses = courses
            .into_iter()
            .map(|(id, long_title, is_current)| {
                Ok(CourseHandle {
                    client: self.client.clone(),
                    meta: CourseMeta {
                        id,
                        long_title,
                        is_current,
                    }
                    .into(),
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        if only_current {
            courses.retain(|c| c.meta.is_current);
        }

        Ok(courses)
    }
}

#[derive(Debug)]
pub struct CourseMeta {
    id: String,
    long_title: String,
    /// 是否是当前学期的课程
    is_current: bool,
}

impl CourseMeta {
    pub fn id(&self) -> &str {
        &self.id
    }

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
            .next_back()
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
        let dom = self.client.bb_coursepage(&self.meta.id).await?;

        let entries = dom
            .select(&Selector::parse("#courseMenuPalette_contents > li > a").unwrap())
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
            &format!("CourseHandle::_get_{}", self.meta.id),
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

#[derive(Debug, Clone)]
pub struct Course {
    client: Client,
    meta: Arc<CourseMeta>,
    entries: HashMap<String, String>,
}

impl Course {
    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn meta(&self) -> &CourseMeta {
        &self.meta
    }

    pub fn content_stream(&self) -> CourseContentStream {
        CourseContentStream::new(
            self.client.clone(),
            self.meta.clone(),
            self.entries()
                .iter()
                .filter_map(|(_, uri)| {
                    let url = low_level::convert_uri(uri).ok()?.into_url().ok()?;
                    if !low_level::blackboard::LIST_CONTENT.ends_with(url.path()) {
                        return None;
                    }

                    let (_, content_id) = url.query_pairs().find(|(k, _)| k == "content_id")?;

                    Some(content_id.to_string())
                })
                .collect(),
        )
    }

    pub fn build_content(&self, data: CourseContentData) -> CourseContent {
        CourseContent {
            client: self.client.clone(),
            course: self.meta.clone(),
            data: data.into(),
        }
    }

    pub fn entries(&self) -> &HashMap<String, String> {
        &self.entries
    }
    #[allow(dead_code)]
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

pub struct CourseContentStream {
    /// 一次性发射的请求数量
    batch_size: usize,
    client: Client,
    course: Arc<CourseMeta>,
    visited_ids: HashSet<String>,
    probe_ids: Vec<String>,
}

impl CourseContentStream {
    fn new(client: Client, course: Arc<CourseMeta>, probe_ids: Vec<String>) -> Self {
        // implicitly deduplicate probe_ids
        let visited_ids = HashSet::from_iter(probe_ids);
        let probe_ids = visited_ids.iter().cloned().collect();
        Self {
            batch_size: 8,
            client,
            course,
            visited_ids,
            probe_ids,
        }
    }
    async fn try_next_batch(&mut self, ids: &[String]) -> anyhow::Result<Vec<CourseContentData>> {
        let futs = ids
            .iter()
            .map(|id| self.client.bb_course_content_page(&self.course.id, id));

        let doms = futures_util::future::join_all(futs).await;

        let mut all_contents = Vec::new();
        for dom in doms {
            let dom = dom?;
            let selector = Selector::parse("#content_listContainer > li").unwrap();
            let contents = dom
                .select(&selector)
                .filter_map(|li| {
                    CourseContentData::from_element(li)
                        .inspect_err(|e| log::warn!("CourseContentData::from_element error: {e}"))
                        .ok()
                })
                // filter out visited ids
                .filter(|data| self.visited_ids.insert(data.id.to_owned()))
                // add the rest new ids to probe_ids
                .inspect(|data| {
                    if data.has_link {
                        self.probe_ids.push(data.id.to_owned())
                    }
                });

            all_contents.extend(contents);
        }

        Ok(all_contents)
    }
    pub async fn next_batch(&mut self) -> Option<Vec<CourseContentData>> {
        let ids = self
            .probe_ids
            .split_off(self.probe_ids.len().saturating_sub(self.batch_size));
        if ids.is_empty() {
            return None;
        }
        match self.try_next_batch(&ids).await {
            Ok(r) => Some(r),
            Err(e) => {
                log::warn!("try_next_batch error {ids:?}: {e}");
                return Box::pin(self.next_batch()).await;
            }
        }
    }
    pub fn num_finished(&self) -> usize {
        self.visited_ids.len() - self.probe_ids.len()
    }
    pub fn len(&self) -> usize {
        self.visited_ids.len()
    }
}

#[derive(Debug, Clone)]
pub struct CourseContent {
    client: Client,
    course: Arc<CourseMeta>,
    data: Arc<CourseContentData>,
}

impl CourseContent {
    pub fn into_assignment_opt(self) -> Option<CourseAssignmentHandle> {
        if let CourseContentKind::Assignment = self.data.kind {
            Some(CourseAssignmentHandle {
                client: self.client,
                course: self.course,
                content: self.data,
            })
        } else {
            None
        }
    }
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
enum CourseContentKind {
    Document,
    Assignment,
    Unknown,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct CourseContentData {
    id: String,
    title: String,
    kind: CourseContentKind,
    has_link: bool,
    descriptions: Vec<String>,
    attachments: Vec<(String, String)>,
}

fn collect_text(element: scraper::ElementRef) -> String {
    let mut text_content = String::new();
    for node_ref in element.children() {
        match node_ref.value() {
            scraper::node::Node::Text(text) => {
                if !text.trim().is_empty() {
                    text_content.push_str(text);
                }
            }
            scraper::node::Node::Element(el) => {
                if el.name() != "script"
                    && let Some(child_element) = scraper::ElementRef::wrap(node_ref)
                {
                    text_content.push_str(&collect_text(child_element));
                }
            }
            _ => {}
        }
    }
    text_content
}

impl CourseContentData {
    fn from_element(el: scraper::ElementRef<'_>) -> anyhow::Result<Self> {
        anyhow::ensure!(el.value().name() == "li", "not a li element");
        let (img, title_div, detail_div) = el.child_elements().take(3).collect_tuple().unwrap();

        let kind = match img.attr("alt") {
            Some("作业") => CourseContentKind::Assignment,
            Some("项目") | Some("文件") => CourseContentKind::Document,
            alt => {
                log::warn!("unknown content kind: {alt:?}");
                CourseContentKind::Unknown
            }
        };

        let id = title_div
            .attr("id")
            .context("content_id not found")?
            .to_owned();

        let title = title_div.text().collect::<String>().trim().to_owned();
        let has_link = title_div
            .select(&Selector::parse("a").unwrap())
            .next()
            .is_some();

        let descriptions = detail_div
            .select(&Selector::parse("div.vtbegenerated > *").unwrap())
            .map(|p| collect_text(p).trim().to_owned())
            .collect::<Vec<_>>();

        let attachments = detail_div
            .select(&Selector::parse("ul.attachments > li > a").unwrap())
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

        Ok(CourseContentData {
            id,
            title,
            kind,
            has_link,
            descriptions,
            attachments,
        })
    }
}

#[derive(Debug, Clone)]
pub struct CourseAssignmentHandle {
    client: Client,
    course: Arc<CourseMeta>,
    content: Arc<CourseContentData>,
}

impl CourseAssignmentHandle {
    pub fn id(&self) -> String {
        let mut hasher = std::hash::DefaultHasher::new();
        self.course.id.hash(&mut hasher);
        self.content.id.hash(&mut hasher);
        let x = hasher.finish();
        format!("{x:x}")
    }

    async fn _get(&self) -> anyhow::Result<CourseAssignmentData> {
        let dom = self
            .client
            .bb_course_assignment_uploadpage(&self.course.id, &self.content.id)
            .await?;

        let deadline = dom
            .select(&Selector::parse("#assignMeta2 + div").unwrap())
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

        Ok(CourseAssignmentData { deadline, attempt })
    }
    pub async fn get(&self) -> anyhow::Result<CourseAssignment> {
        let data = with_cache(
            &format!(
                "CourseAssignmentHandle::_get_{}_{}",
                self.content.id, self.course.id
            ),
            self.client.cache_ttl(),
            self._get(),
        )
        .await?;

        Ok(CourseAssignment {
            client: self.client.clone(),
            course: self.course.clone(),
            content: self.content.clone(),
            data,
        })
    }

    async fn _get_current_attempt(&self) -> anyhow::Result<Option<String>> {
        let dom = self
            .client
            .bb_course_assignment_viewpage(&self.course.id, &self.content.id)
            .await?;

        let attempt_label = if let Some(e) = dom
            .select(&Selector::parse("h3#currentAttempt_label").unwrap())
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

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
struct CourseAssignmentData {
    // descriptions: Vec<String>,
    // attachments: Vec<(String, String)>,
    deadline: Option<String>,
    attempt: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CourseAssignment {
    client: Client,
    course: Arc<CourseMeta>,
    content: Arc<CourseContentData>,
    data: CourseAssignmentData,
}

impl CourseAssignment {
    pub fn title(&self) -> &str {
        &self.content.title
    }

    pub fn descriptions(&self) -> &[String] {
        &self.content.descriptions
    }

    pub fn attachments(&self) -> &[(String, String)] {
        &self.content.attachments
    }

    pub fn last_attempt(&self) -> Option<&str> {
        self.data.attempt.as_deref()
    }

    pub async fn get_submit_formfields(&self) -> anyhow::Result<HashMap<String, String>> {
        let dom = self
            .client
            .bb_course_assignment_uploadpage(&self.course.id, &self.content.id)
            .await?;

        let extract_field = |input: scraper::ElementRef<'_>| {
            let name = input.value().attr("name")?.to_owned();
            let value = input.value().attr("value")?.to_owned();
            Some((name, value))
        };

        let submitformfields = dom
            .select(&Selector::parse("form#uploadAssignmentFormId input").unwrap())
            .map(extract_field)
            .chain(
                dom.select(&Selector::parse("div.field input").unwrap())
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

        let res = self.client.bb_course_assignment_uploaddata(body).await?;

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
            .bb_course_video_sub_info(&course_id, &sub_id, &app_id, &auth_data)
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

        if let Some(newkey) = &seg.key
            && key.is_none_or(|k| fallback_keyformat(k) == fallback_keyformat(newkey))
        {
            return Some(newkey);
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

#[derive(Debug)]
pub struct Syllabus {
    client: Client,
    username: String,
}

impl Syllabus {
    /// 获取选课结果
    pub async fn get_results(&self) -> anyhow::Result<Vec<SyllabusBaseCourseData>> {
        let dom = self.client.sb_resultspage().await?;
        let table_sel = Selector::parse("table.datagrid").unwrap();
        let table = dom.select(&table_sel).nth(0).context("table not found")?;
        let tbody = table
            .child_elements()
            .nth(0)
            .context("table tbody not found")?;

        let mut rows = tbody.child_elements();
        let header_row = rows.next().context("table header not found")?;
        anyhow::ensure!(
            header_row.value().name() == "tr",
            "header not tr, got {}",
            header_row.value().name()
        );

        let col_names = header_row
            .child_elements()
            .map(|el| el.text().collect::<String>().trim().to_owned())
            .collect::<Vec<_>>();

        anyhow::ensure!(
            col_names
                == [
                    "课程名",
                    "课程类别",
                    "学分",
                    "周学时",
                    "教师",
                    "班号",
                    "开课单位",
                    "教室信息",
                    "自选P/NP",
                    "选课结果",
                    "IP地址",
                    "操作时间",
                ],
            "unexpected column names: {:?}",
            col_names
        );

        let mut r = Vec::new();
        for row in rows {
            // 对应 "Page 1 of 1  First / Previous   Next / Last" 这一行
            if row.child_elements().count() <= 1 {
                continue;
            }
            let row_values = row
                .child_elements()
                .map(|el| el.text().collect::<String>().trim().to_owned())
                .collect::<Vec<_>>();
            r.push(SyllabusBaseCourseData {
                name: row_values[0].to_owned(),
                category: row_values[1].to_owned(),
                score: row_values[2].to_owned(),
                hours_per_week: row_values[3].to_owned(),
                teacher: row_values[4].to_owned(),
                class_id: row_values[5].to_owned(),
                department: row_values[6].to_owned(),
                classroom: row_values[7].to_owned(),
                custom_n_or_np: row_values[8].to_owned(),
                status: row_values[9].to_owned(),
            });
        }
        Ok(r)
    }

    /// 获取补选总页数，必须在获取补选课程前调用，否则会返回空页面
    pub async fn get_supplements_total_pages(&self) -> anyhow::Result<usize> {
        let dom = self.client.sb_supplycancelpage(&self.username).await?;
        let pagination_sel = Selector::parse("tr[align=\"right\"] > td:first-child").unwrap();
        let re = regex::Regex::new(r"Page\s*\d+?\s*of\s*(\d+?)").unwrap();

        let td = dom
            .select(&pagination_sel)
            .nth(0)
            .context("table footer not found")?;
        let text = td.text().collect::<String>();
        let m = re.captures(&text).context("page count not matched")?;

        let total: usize = m.get(1).context("page count not found")?.as_str().parse()?;

        Ok(total)
    }

    pub async fn get_supplements(
        &self,
        page: usize,
    ) -> anyhow::Result<Vec<SyllabusSupplementCourseData>> {
        let dom = self.client.sb_supplementpage(&self.username, page).await?;
        let table_sel = Selector::parse("table.datagrid").unwrap();
        let table = dom.select(&table_sel).nth(0).context("table not found")?;
        let tbody = table
            .child_elements()
            .nth(0)
            .context("table tbody not found")?;

        let mut rows = tbody.child_elements();
        let header_row = rows.next().context("table header not found")?;
        anyhow::ensure!(
            header_row.value().name() == "tr",
            "header not tr, got {}",
            header_row.value().name()
        );

        let col_names = header_row
            .child_elements()
            .map(|el| el.text().collect::<String>().trim().to_owned())
            .collect::<Vec<_>>();

        anyhow::ensure!(
            col_names
                == [
                    "课程名",
                    "课程类别",
                    "学分",
                    "周学时",
                    "教师",
                    "班号",
                    "开课单位",
                    "年级",
                    "上课/考试信息",
                    "自选P/NP",
                    "限数/已选/候补",
                    "补选",
                ],
            "unexpected column names: {:?}",
            col_names
        );

        let mut r = Vec::new();
        for row in rows {
            // 对应 "Page 1 of 1  First / Previous   Next / Last" 这一行
            if row.child_elements().count() <= 2 {
                continue;
            }
            let row_values = row
                .child_elements()
                .map(|el| el.text().collect::<String>().trim().to_owned())
                .collect::<Vec<_>>();
            r.push(SyllabusSupplementCourseData {
                base: SyllabusBaseCourseData {
                    name: row_values[0].to_owned(),
                    category: row_values[1].to_owned(),
                    score: row_values[2].to_owned(),
                    hours_per_week: row_values[3].to_owned(),
                    teacher: row_values[4].to_owned(),
                    class_id: row_values[5].to_owned(),
                    department: row_values[6].to_owned(),
                    classroom: row_values[8].to_owned(),
                    custom_n_or_np: row_values[9].to_owned(),
                    status: row_values[10].to_owned(),
                },
                supplement_url: row
                    .child_elements()
                    .last()
                    .unwrap()
                    .child_elements()
                    .nth(0)
                    .context("<a> not found")?
                    .attr("href")
                    .context("supplement url not found")?
                    .to_string(),
                page_id: page,
            });
        }
        Ok(r)
    }

    /// 尝试补选一门课
    pub async fn elect(
        &self,
        course: &SyllabusSupplementCourseData,
        ttshitu_username: String,
        ttshitu_password: String,
    ) -> anyhow::Result<bool> {
        loop {
            let image = self.client.sb_draw_servlet().await?;
            log::trace!("captcha image size: {} bytes", image.len());

            let image_b64 = crate::ttshitu::jpeg_to_b64(&image)?;
            log::trace!("captcha image base64 size: {} chars", image_b64.len());

            let code = self
                .client
                .ttshitu_recognize(
                    ttshitu_username.clone(),
                    ttshitu_password.clone(),
                    image_b64,
                )
                .await?;
            log::debug!("captcha code recognition: {}", code);

            let r = self
                .client
                .sb_send_validation(&self.username, &code)
                .await?;
            log::trace!("captcha validation response: {}", r);
            if r == 2 {
                break;
            }
            log::warn!("验证码不正确，正在重试...");
        }

        match self
            .client
            .sb_elect_by_url(&format!(
                "https://elective.pku.edu.cn{}",
                course.supplement_url
            ))
            .await
        {
            Ok(()) => {
                log::info!("{} 选择完成", course.name);
                Ok(true)
            }
            Err(e) => {
                log::warn!("{} 选择失败: {:#}", course.name, e);
                Ok(false)
            }
        }
    }
}

#[derive(Debug)]
#[allow(unused)]
pub struct SyllabusBaseCourseData {
    /// 课程名
    pub name: String,
    /// 课程类别
    pub category: String,
    /// 学分
    pub score: String,
    /// 周学时
    pub hours_per_week: String,
    /// 教师
    pub teacher: String,
    /// 班号
    pub class_id: String,
    /// 开课单位
    pub department: String,
    /// 教室信息
    pub classroom: String,
    /// 自选P/NP
    pub custom_n_or_np: String,
    /// 选课结果 or 限数/已选/候补 or 限数/已选
    pub status: String,
}

impl SyllabusBaseCourseData {
    pub fn is_full(&self) -> anyhow::Result<bool> {
        status_is_full(&self.status)
    }
}

fn status_is_full(status: &str) -> anyhow::Result<bool> {
    let tokens = status.split('/').collect::<Vec<_>>();
    match tokens.as_slice() {
        [limit, selected] => {
            let limit: usize = limit.trim().parse()?;
            let selected: usize = selected.trim().parse()?;
            Ok(selected >= limit)
        }
        _ => anyhow::bail!("unexpected status format: {}", status),
    }
}

#[derive(Debug)]
pub struct SyllabusSupplementCourseData {
    pub base: SyllabusBaseCourseData,
    /// 补选操作 URL
    pub supplement_url: String,
    /// 课程位于补退选的第几页
    pub page_id: usize,
}

impl std::ops::Deref for SyllabusSupplementCourseData {
    type Target = SyllabusBaseCourseData;
    fn deref(&self) -> &Self::Target {
        &self.base
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

    #[test]
    fn test_status_is_full() {
        assert_eq!(status_is_full("30 /25 ").unwrap(), false);
        assert_eq!(status_is_full(" 30/ 30").unwrap(), true);
        assert_eq!(status_is_full("30 / 35 ").unwrap(), true);
        assert!(status_is_full("invalid").is_err());
    }
}
