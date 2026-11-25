mod video;

use super::*;
use crate::api::low_level::blackboard::BlackboardUnautherizedError;
use serde::Deserialize;
pub use video::{CourseVideo, CourseVideoSource};

impl Client {
    /// 已保存的教学网会话是否仍然有效。只访问教学网，不访问 IAAA。
    pub async fn blackboard_session_valid(&self) -> bool {
        match self.0.http_client.bb_homepage().await {
            Ok(_) => {
                log::info!("reuse saved login session");
                true
            }
            Err(e) => {
                if let Err(e) = e.downcast::<BlackboardUnautherizedError>() {
                    log::error!("error during preflight: {e}");
                }
                false
            }
        }
    }

    /// `need_login` 为 false 时直接复用已保存的会话（调用方应已用
    /// [`Self::blackboard_session_valid`] 确认有效），否则重新登录。
    pub async fn blackboard(
        &self,
        username: &str,
        password: &str,
        otp_code: &str,
        need_login: bool,
    ) -> anyhow::Result<Blackboard> {
        let c = &self.0.http_client;
        if need_login {
            c.bb_login(username, password, otp_code).await?;

            // Warm up both web and REST API sessions after login
            c.bb_homepage().await?;

            if let Some(path) = &self.0.cookie_restore_path {
                c.save_set_cookies(path).await?;
                log::info!("blackboard login session saved to {}", path.display());
            }
        }

        Ok(Blackboard {
            client: self.clone(),
        })
    }

    pub async fn course_attachment_download<P: AsRef<std::path::Path>>(
        &self,
        uri: &str,
        dest: P,
        redir: bool,
    ) -> anyhow::Result<()> {
        log::debug!("downloading attachment from {uri}");
        let uri = if redir {
            let res = self.get_by_uri(uri).await?;
            let loc = low_level::extract_redirect_url(&res)?;
            log::debug!("redirected to {loc}");
            loc.to_owned()
        } else {
            uri.to_owned()
        };

        let res = self.get_by_uri(&uri).await?;
        anyhow::ensure!(res.status().is_success(), "status not success");

        let rbody = res.bytes().await?;
        let r = compio::fs::write(dest, rbody).await;
        compio::buf::buf_try!(@try r);
        Ok(())
    }
}

#[derive(Debug)]
pub struct Blackboard {
    client: Client,
    // token: String,
}

/// 从教学网主页解析课程列表，返回 `(course key, 标题, 是否当前学期)`。
///
/// 课程栏目里不是课程的链接（没有 `key=`）会被跳过，而不是让整个列表失败 (#51)。
fn parse_courses(dom: &scraper::Html) -> anyhow::Result<Vec<(String, String, bool)>> {
    let re = regex::Regex::new(r"key=([\d_]+),").unwrap();
    let portlet_sel = Selector::parse("div.portlet").unwrap();
    let title_in_portlet_sel = Selector::parse("span.moduleTitle").unwrap();
    let ul_sel = Selector::parse("ul.courseListing").unwrap();
    let sel = Selector::parse("li a").unwrap();

    // use regex to extract course key (of form key=_80052_1)
    let to_key_text = |a: scraper::ElementRef<'_>| {
        let text = a.text().collect::<String>();
        let key = a
            .value()
            .attr("href")
            .and_then(|href| re.captures(href))
            .and_then(|s| s.get(1));
        match key {
            Some(key) => Some((key.as_str().to_owned(), text)),
            None => {
                log::warn!("skipping link without course key: {text}");
                None
            }
        }
    };

    // the first one contains the courses in the current semester
    // the second one contains the courses in the previous semester
    let mut courses = Vec::new();

    for portlet in dom.select(&portlet_sel) {
        let Some(title) = portlet.select(&title_in_portlet_sel).next() else {
            continue;
        };
        let title = title.text().collect::<String>();
        log::info!("scanning portlet: {title}");

        if !title.contains("课程") && !title.contains("Courses") {
            continue;
        }

        let is_current = title.contains("当前") || title.contains("Current Semester Courses");
        for ul in portlet.select(&ul_sel) {
            let items = ul
                .select(&sel)
                .filter_map(to_key_text)
                .map(|(k, t)| (k, t, is_current))
                .collect::<Vec<_>>();
            log::info!("found {} courses, is_current: {is_current}", items.len());
            courses.extend(items);
        }
    }

    if courses.is_empty() {
        anyhow::bail!("courses not found");
    }

    Ok(courses)
}

impl Blackboard {
    async fn _get_courses(&self) -> anyhow::Result<Vec<(String, String, bool)>> {
        let dom = self.client.bb_homepage().await?;
        parse_courses(&dom)
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

    pub async fn user_info_id(&self) -> anyhow::Result<String> {
        #[derive(Debug, Deserialize)]
        struct UserInfo {
            id: String,
        }

        let user_info: UserInfo = self
            .client
            .0
            .http_client
            .api_get("https://course.pku.edu.cn/learn/api/public/v1/users/me")
            .await
            .map_err(|e| {
                if format!("{e}").contains("401") {
                    anyhow::anyhow!("session expired — use -f or --force to re-login ({e})")
                } else {
                    e
                }
            })
            .context("fetch user info")?;
        Ok(user_info.id)
    }

    pub async fn user_courses(&self, user_id: &str) -> anyhow::Result<Vec<CourseEnrollment>> {
        #[derive(Debug, Deserialize)]
        struct Result {
            results: Vec<CourseEnrollment>,
        }
        let val: serde_json::Value = self
            .client
            .0
            .http_client
            .api_get(&format!(
                "https://course.pku.edu.cn/learn/api/public/v1/users/{}/courses?expand=course",
                user_id
            ))
            .await
            .context("fetch user courses")?;
        let val: Result = serde_json::from_value(val)?;
        Ok(val.results)
    }

    pub async fn course_detail(&self, course_id: &str) -> anyhow::Result<CourseDetailHandle> {
        let val: CourseDetail = self
            .client
            .0
            .http_client
            .api_get(&format!(
                "https://course.pku.edu.cn/learn/api/public/v1/courses/{}",
                course_id
            ))
            .await
            .context("fetch user courses")?;
        Ok(CourseDetailHandle {
            client: self.client.clone(),
            id: course_id.to_owned(),
            data: val,
        })
    }

    pub async fn get_ta_courses(&self, user_id: &str) -> anyhow::Result<Vec<CourseEnrollment>> {
        let enrollments = self.user_courses(user_id).await?;
        Ok(enrollments
            .into_iter()
            .filter(|e| e.course_role_id == "TeachingAssistant")
            .collect())
    }

    pub async fn get_course_groups(&self, course_id: &str) -> anyhow::Result<Vec<CourseGroup>> {
        #[derive(Debug, Deserialize)]
        struct Wrapper {
            results: Vec<CourseGroup>,
        }
        let val: Wrapper = self
            .client
            .0
            .http_client
            .api_get(&format!(
                "https://course.pku.edu.cn/learn/api/public/v1/courses/{}/groups",
                course_id
            ))
            .await
            .context("fetch course groups")?;
        Ok(val.results)
    }

    pub async fn get_group_users(
        &self,
        course_id: &str,
        group_id: &str,
    ) -> anyhow::Result<Vec<String>> {
        #[derive(Debug, Deserialize)]
        struct UserRef {
            #[serde(rename = "userId")]
            user_id: String,
        }
        let val: serde_json::Value = self
            .client
            .0
            .http_client
            .api_get(&format!(
                "https://course.pku.edu.cn/learn/api/public/v2/courses/{}/groups/{}/users",
                course_id, group_id
            ))
            .await
            .context("fetch group users")?;
        // v2 API may return {results:[{userId:...}]} or just [{userId:...}]
        let users: Vec<UserRef> = if let Some(results) = val.get("results") {
            serde_json::from_value(results.clone())?
        } else {
            serde_json::from_value(val)?
        };
        Ok(users.into_iter().map(|u| u.user_id).collect())
    }

    pub async fn get_course_memberships(
        &self,
        course_id: &str,
    ) -> anyhow::Result<Vec<CourseMembership>> {
        let val: serde_json::Value = self
            .client
            .0
            .http_client
            .api_get(&format!(
                "https://course.pku.edu.cn/learn/api/public/v1/courses/{}/users",
                course_id
            ))
            .await
            .context("fetch course memberships")?;
        #[derive(Deserialize)]
        struct Wrapper {
            results: Vec<CourseMembership>,
        }
        let w: Wrapper = serde_json::from_value(val)?;
        Ok(w.results)
    }

    pub async fn get_user_name(&self, user_id: &str) -> anyhow::Result<String> {
        #[derive(Deserialize)]
        struct UserName {
            given: String,
        }
        #[derive(Deserialize)]
        struct UserInfo {
            name: UserName,
        }
        let info: UserInfo = self
            .client
            .0
            .http_client
            .api_get(&format!(
                "https://course.pku.edu.cn/learn/api/public/v1/users/{}",
                user_id
            ))
            .await
            .context("fetch user name")?;
        Ok(info.name.given)
    }

    pub async fn download_attempt_file(&self, download_url: &str) -> anyhow::Result<bytes::Bytes> {
        self.client.bb_download_attempt_file(download_url).await
    }

    pub async fn load_reconcile_data(
        &self,
        course_id: &str,
        column_id: &str,
    ) -> anyhow::Result<ReconcileData> {
        let body = self
            .client
            .bb_load_reconcile_data(course_id, column_id)
            .await?;
        let data: ReconcileData = serde_json::from_str(&body).context("parse reconcile data")?;
        Ok(data)
    }

    pub async fn get_reconcile_nonce(
        &self,
        course_id: &str,
        column_id: &str,
    ) -> anyhow::Result<String> {
        let html = self.client.bb_reconcile_page(course_id, column_id).await?;
        let html_str = html.root_element().html();
        let nonce = regex::Regex::new(
            r#"name="blackboard\.platform\.security\.NonceUtil\.nonce\.ajax"[^>]*value="([^"]+)""#,
        )
        .unwrap()
        .captures(&html_str)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_owned())
        .context("nonce not found in reconcile page")?;
        Ok(nonce)
    }

    pub async fn save_grade(
        &self,
        attempt_id: &str,
        gradable_item_id: &str,
        score: f64,
        course_id: &str,
        nonce: &str,
        feedback: Option<&str>,
    ) -> anyhow::Result<()> {
        self.client
            .bb_save_grade(
                attempt_id,
                gradable_item_id,
                score,
                course_id,
                nonce,
                feedback,
            )
            .await?;
        Ok(())
    }

    pub async fn get_attempt_file_info(
        &self,
        attempt_id: &str,
        course_id: &str,
        column_id: &str,
        membership_id: &str,
    ) -> anyhow::Result<AttemptFileInfo> {
        let html = self
            .client
            .bb_grading_page(column_id, course_id, attempt_id, membership_id)
            .await?;

        let html_str = html.root_element().html();
        let file_id = regex::Regex::new(r"inlineView\(\s*event\s*,\s*'([^']+)'")
            .unwrap()
            .captures(&html_str)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_owned());

        let Some(file_id) = file_id else {
            return Ok(AttemptFileInfo::NoFile);
        };

        let json_text = self
            .client
            .bb_inline_view(&file_id, attempt_id, course_id)
            .await?;

        #[derive(Deserialize)]
        struct InlineViewResponse {
            #[serde(rename = "downloadUrl")]
            download_url: Option<String>,
            #[serde(rename = "fileName")]
            file_name: Option<String>,
        }
        let iv: InlineViewResponse =
            serde_json::from_str(&json_text).context("parse inlineView response")?;

        Ok(AttemptFileInfo::File {
            download_url: iv.download_url.unwrap_or_default(),
            file_name: iv.file_name.unwrap_or_default(),
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct CourseGroup {
    pub id: String,
    pub name: String,
    #[serde(rename = "isGroupSet")]
    pub is_group_set: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CourseMembership {
    pub id: String,
    #[serde(rename = "userId")]
    pub user_id: String,
}

pub enum AttemptFileInfo {
    NoFile,
    File {
        download_url: String,
        file_name: String,
    },
}

#[derive(Debug, Deserialize)]
pub struct ReconcileData {
    #[serde(default)]
    pub attempts: Vec<ReconcileAttempt>,
}

#[derive(Debug, Deserialize)]
pub struct ReconcileAttempt {
    #[serde(rename = "attemptId")]
    pub attempt_id: String,
    #[serde(rename = "studentUserId")]
    pub student_user_id: String,
    pub status: String,
    #[serde(default)]
    #[serde(rename = "reconciledScore")]
    pub _reconciled_score: Option<f64>,
    #[serde(default)]
    #[serde(rename = "provisionalGrades")]
    pub provisional_grades: Vec<ProvisionalGrade>,
}

#[derive(Debug, Deserialize)]
pub struct ProvisionalGrade {
    #[serde(rename = "graderUserId")]
    #[serde(default)]
    pub _grader_user_id: Option<String>,
    pub score: Option<f64>,
    #[serde(rename = "status")]
    pub _status: String,
}

#[derive(Debug, Deserialize)]
pub struct CourseEnrollment {
    #[serde(rename = "courseId")]
    pub course_id: String,
    #[serde(rename = "courseRoleId")]
    pub course_role_id: String,
    #[serde(default)]
    #[serde(rename = "course")]
    pub course: Option<CourseBrief>,
}

#[derive(Debug, Deserialize)]
pub struct CourseBrief {
    #[serde(rename = "name")]
    pub name: String,
    #[serde(default)]
    #[serde(rename = "term")]
    pub term: Option<TermBrief>,
}

#[derive(Debug, Deserialize)]
pub struct TermBrief {
    #[serde(rename = "name")]
    pub name: String,
}

pub struct CourseDetailHandle {
    client: Client,
    id: String,
    data: CourseDetail,
}

impl CourseDetailHandle {
    pub fn data(&self) -> &CourseDetail {
        &self.data
    }

    pub async fn gradebook_columns(&self) -> anyhow::Result<Vec<GradebookColumn>> {
        #[derive(Debug, Deserialize)]
        struct GradebookColumns {
            results: Vec<GradebookColumn>,
        }

        let val: GradebookColumns = self
            .client
            .0
            .http_client
            .api_get(&format!(
                "https://course.pku.edu.cn/learn/api/public/v2/courses/{}/gradebook/columns",
                self.id
            ))
            .await
            .context("fetch gradebook columns")?;
        Ok(val.results)
    }

    pub async fn gradedata(&self, column_id: &str) -> anyhow::Result<Vec<GradeUser>> {
        #[derive(Debug, Deserialize)]
        struct GradeUsers {
            results: Vec<GradeUser>,
        }

        let val: GradeUsers = self
            .client
            .0
            .http_client
            .api_get(&format!(
                "https://course.pku.edu.cn/learn/api/public/v2/courses/{}/gradebook/columns/{}/users",
                self.id, column_id
            ))
            .await
            .context("fetch gradebook columns")?;
        Ok(val.results)
    }

    pub async fn get_attempts(&self, column_id: &str) -> anyhow::Result<Vec<AttemptRecord>> {
        #[derive(Debug, Deserialize)]
        struct AttemptsResult {
            results: Vec<AttemptRecord>,
        }

        let val: AttemptsResult = self
            .client
            .0
            .http_client
            .api_get(&format!(
                "https://course.pku.edu.cn/learn/api/public/v2/courses/{}/gradebook/columns/{}/attempts",
                self.id, column_id
            ))
            .await
            .context("fetch attempts")?;
        Ok(val.results)
    }

    pub async fn all_grades(&self) -> anyhow::Result<Vec<GradeRecord>> {
        let columns = self.gradebook_columns().await?;

        let mut all_grades = Vec::new();
        for col in &columns {
            if let Some(grading) = &col.grading
                && grading.grading_type == "Calculated"
                && col.name.contains("总计")
                && !col.name.contains("平时")
            {
                continue;
            }

            let grade_data = match self.gradedata(&col.id).await {
                Ok(data) => data.into_iter().next(),
                Err(_) => None,
            };

            let possible = col.score.as_ref().map(|s| s.possible).unwrap_or(0.0);
            let score = grade_data
                .and_then(|g| g.display_grade)
                .and_then(|d| d.score);

            all_grades.push(GradeRecord {
                course_name: self.data.name.clone(),
                column_name: col.name.clone(),
                score,
                possible,
            });
        }
        Ok(all_grades)
    }
}

#[derive(Debug, Deserialize)]
pub struct CourseDetail {
    name: String,
    availability: Option<Availability>,
}

impl CourseDetail {
    pub fn is_available(&self) -> bool {
        self.availability
            .as_ref()
            .is_some_and(|a| a.available == "Yes")
    }
}

#[derive(Debug, Deserialize)]
struct Availability {
    pub available: String,
}

#[derive(Debug, Deserialize)]
pub struct GradebookColumn {
    pub id: String,
    pub name: String,
    pub score: Option<ColumnScore>,
    pub grading: Option<Grading>,
}

#[derive(Debug, Deserialize)]
pub struct ColumnScore {
    pub possible: f64,
}

#[derive(Debug, Deserialize)]
pub struct Grading {
    #[serde(rename = "type")]
    pub grading_type: String,
}

#[derive(Debug, Deserialize)]
pub struct GradeUser {
    #[serde(rename = "userId")]
    pub user_id: String,
    #[serde(rename = "displayGrade")]
    pub display_grade: Option<DisplayGrade>,
    pub status: Option<String>,
    #[serde(default)]
    pub exempt: bool,
}

#[derive(Debug, Deserialize)]
pub struct AttemptRecord {
    pub id: String,
    #[serde(rename = "userId")]
    pub user_id: String,
    pub status: Option<String>,
    #[serde(rename = "attemptDate")]
    pub attempt_date: Option<String>,
    #[serde(default)]
    pub exempt: bool,
}

#[derive(Debug, Deserialize)]
pub struct DisplayGrade {
    pub score: Option<f64>,
}

#[derive(Debug)]
pub struct GradeRecord {
    pub course_name: String,
    pub column_name: String,
    pub score: Option<f64>,
    pub possible: f64,
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
        self.long_title
            .split_once(':')
            .map(|(_, s)| s.trim())
            .unwrap_or(self.long_title.trim())
    }

    /// Cousre Name
    pub fn name(&self) -> &str {
        let s = self.title();
        s.char_indices()
            .rfind(|(_, c)| *c == '(')
            .map(|(i, _)| s.split_at(i).0.trim())
            .unwrap_or(s)
    }
}

#[derive(Debug, Clone)]
pub struct CourseHandle {
    client: Client,
    meta: Arc<CourseMeta>,
}

impl CourseHandle {
    pub fn id(&self) -> &str {
        &self.meta.id
    }

    pub fn long_title(&self) -> &str {
        &self.meta.long_title
    }

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
                .values()
                .filter_map(|uri| {
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

    /// 直接从课程公告页抓取课程公告。
    pub async fn list_announcements_from_coursepage(
        &self,
    ) -> anyhow::Result<Vec<CourseAnnouncementHandle>> {
        log::info!(
            "fetching announcement list from course page for {}",
            self.meta.title()
        );

        let dom = self.client.bb_coursepage(&self.meta.id).await?;
        let container_selector =
            Selector::parse(".vtbegenerated, #content_listContainer, div.content, div.clearfix")
                .unwrap();
        let h3_selector = Selector::parse("h3").unwrap();

        let mut parsed_announcements = Vec::new();

        for container in dom.select(&container_selector) {
            let h3_elements = container.select(&h3_selector).collect::<Vec<_>>();

            if !h3_elements.is_empty() {
                for h3 in h3_elements {
                    let title = h3.text().collect::<String>().trim().to_string();

                    if title.is_empty()
                        || title.contains("课程")
                        || title.contains("学期")
                        || title == "我的小组"
                        || title == "公告"
                        || title.contains("查看选项")
                        || title.contains("菜单管理")
                    {
                        continue;
                    }

                    let mut sibling = h3.next_sibling();
                    let mut content = String::new();
                    let mut time = String::new();

                    for _ in 0..10 {
                        let Some(sib) = sibling else {
                            break;
                        };

                        if let Some(elem) = sib.value().as_element() {
                            let el_ref = scraper::ElementRef::wrap(sib).unwrap();
                            let tag = elem.name();

                            if tag == "h3" {
                                break;
                            }

                            let text = el_ref.text().collect::<String>();
                            if tag == "p" && text.contains("发布") {
                                time = text.trim().to_string();
                            } else if (tag == "div" || tag == "p") && !text.trim().is_empty() {
                                if !content.is_empty() {
                                    content.push('\n');
                                }
                                content.push_str(&text);
                            }
                        }

                        sibling = sib.next_sibling();
                    }

                    parsed_announcements.push((title, content, time));
                }
            } else {
                let content = container.text().collect::<String>().trim().to_string();
                let time = container
                    .select(&Selector::parse("p").unwrap())
                    .next()
                    .map(|el| el.text().collect::<String>().trim().to_string())
                    .unwrap_or_default();

                let lower_content = content.to_lowercase();
                if lower_content.contains("var json")
                    || lower_content.contains("查看选项")
                    || lower_content.contains("菜单管理")
                {
                    continue;
                }

                if !content.is_empty() && content.len() > 10 {
                    let title = content.chars().take(20).collect::<String>();
                    let title = if content.chars().count() > 20 {
                        format!("{title}...")
                    } else {
                        title
                    };
                    parsed_announcements.push((title, content, time));
                }
            }
        }

        let mut announcements = Vec::new();
        let mut seen_titles = HashSet::new();

        for (idx, (title, content, time)) in parsed_announcements.iter().enumerate() {
            if title.is_empty() || title.len() < 5 {
                continue;
            }

            let course_name = self.meta.name();
            if title.starts_with(course_name) || title.contains("学期") || title == "公告" {
                continue;
            }

            let content_clean = content.trim();
            if content_clean.starts_with(course_name) && content_clean.len() < 50 {
                continue;
            }

            let dedup_key = announcement_dedup_key(title, content, time);
            if !seen_titles.insert(format!("{}:{dedup_key}", self.meta.id)) {
                continue;
            }

            let id = format!("{}_{}", self.meta.id, idx);
            let content_data = CourseContentData {
                id: id.clone(),
                title: title.clone(),
                kind: CourseContentKind::Announcement,
                has_link: false,
                descriptions: if !content.is_empty() {
                    content
                        .lines()
                        .map(str::trim)
                        .filter(|line| !line.is_empty())
                        .map(ToOwned::to_owned)
                        .collect()
                } else {
                    vec![]
                },
                attachments: vec![],
                time: if !time.is_empty() {
                    Some(time.clone())
                } else {
                    None
                },
            };

            announcements.push(CourseAnnouncementHandle {
                course: self.meta.clone(),
                content: Arc::new(content_data),
            });
        }

        log::info!(
            "found {} announcements for course {}",
            announcements.len(),
            self.meta.title()
        );

        Ok(announcements)
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
        log::debug!("try_next_batch: {ids:?}");
        let futs = ids
            .iter()
            .map(|id| self.client.bb_course_content_page(&self.course.id, id));

        let doms = futures_util::future::join_all(futs).await;

        let mut all_contents = Vec::new();
        for dom in doms {
            let dom = dom?;
            let contents = parse_content_list(&dom)
                .into_iter()
                // filter out visited ids
                .filter(|data| self.visited_ids.insert(data.id.to_owned()))
                // add the rest new ids to probe_ids
                .inspect(|data| {
                    log::debug!(
                        "find new content {:?}, title = {}, kind = {:?}",
                        data.id,
                        data.title,
                        data.kind
                    );
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

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CourseContentID {
    course_id: String,
    content_id: String,
}

impl CourseContentID {
    pub fn course_id(&self) -> &str {
        &self.course_id
    }
    pub fn content_id(&self) -> &str {
        &self.content_id
    }
}

impl std::fmt::Display for CourseContentID {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.course_id, self.content_id)
    }
}

impl std::str::FromStr for CourseContentID {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (course_id, content_id) = s.split_once(':').context("invalid course content id")?;
        Ok(CourseContentID {
            course_id: course_id.to_owned(),
            content_id: content_id.to_owned(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct CourseContent {
    client: Client,
    course: Arc<CourseMeta>,
    data: Arc<CourseContentData>,
}

impl CourseContent {
    pub fn title(&self) -> &str {
        &self.data.title
    }

    pub fn ccid(&self) -> CourseContentID {
        CourseContentID {
            course_id: self.course.id.clone(),
            content_id: self.data.id.clone(),
        }
    }

    pub fn kind(&self) -> &CourseContentKind {
        &self.data.kind
    }

    pub fn attachments(&self) -> &[(String, String)] {
        &self.data.attachments
    }

    pub fn descriptions(&self) -> &[String] {
        &self.data.descriptions
    }

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

/// 解析内容页（listContent.jsp）的条目列表。
///
/// 只取页面上第一个 `#content_listContainer` 的直接子 `li`：条目描述里可能粘贴了
/// 教学网页面的 HTML，带有同 id 的嵌套列表，它们不是本页的条目 (#43)。
fn parse_content_list(dom: &scraper::Html) -> Vec<CourseContentData> {
    let selector = Selector::parse("#content_listContainer").unwrap();
    dom.select(&selector)
        .next()
        .into_iter()
        .flat_map(|ul| ul.child_elements())
        .filter(|el| el.value().name() == "li")
        .filter_map(|li| {
            CourseContentData::from_element(li)
                .inspect_err(|e| log::warn!("CourseContentData::from_element error: {e}"))
                .ok()
        })
        .collect()
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub enum CourseContentKind {
    Document,
    File,
    Assignment,
    Announcement,
    Audio,
    Folder,
    Quiz,
    WebLink,
    Image,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    time: Option<String>,
}

fn collect_text(element: scraper::ElementRef) -> String {
    let mut text_content = String::new();
    for node_ref in element.children() {
        match node_ref.value() {
            scraper::node::Node::Text(text) if !text.trim().is_empty() => {
                text_content.push_str(text);
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

fn normalize_compact_text(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

fn announcement_dedup_key(title: &str, content: &str, time: &str) -> String {
    let title = normalize_compact_text(title);
    let content = normalize_compact_text(content);
    let time = normalize_compact_text(time);

    if content.is_empty() {
        format!("{title}|{time}")
    } else {
        format!("{title}|{time}|{content}")
    }
}

impl CourseContentData {
    fn from_element(el: scraper::ElementRef<'_>) -> anyhow::Result<Self> {
        anyhow::ensure!(el.value().name() == "li", "not a li element");
        let (img, title_div, detail_div) = el
            .child_elements()
            .take(3)
            .collect_tuple()
            .context("failed to collect 3 child elements")?;

        let kind = match img.attr("alt") {
            Some("作业") => CourseContentKind::Assignment,
            Some("音频") => CourseContentKind::Audio,
            Some("内容文件夹") => CourseContentKind::Folder,
            Some("项目") => CourseContentKind::Document,
            Some("文件") => CourseContentKind::File,
            Some("测试") => CourseContentKind::Quiz,
            Some("Web 链接") => CourseContentKind::WebLink,
            Some("图像") => CourseContentKind::Image,
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
        // 只有指向内容页的链接（文件夹）才需要继续展开；文件、作业等条目的标题
        // 也是链接，但用它们的 id 请求内容页只会拿回所在文件夹的同一页 (#43)
        let has_link = title_div
            .select(&Selector::parse("a").unwrap())
            .filter_map(|a| a.attr("href"))
            .any(|href| href.contains("/webapps/blackboard/content/listContent"));

        let descriptions = detail_div
            .select(&Selector::parse("div.vtbegenerated > *").unwrap())
            .map(|p| collect_text(p).trim().to_owned())
            .collect::<Vec<_>>();

        let mut attachments = detail_div
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

        let audio = detail_div
            .select(&Selector::parse("audio + ul > li > a").unwrap())
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

        attachments.extend(audio);

        Ok(CourseContentData {
            id,
            title,
            kind,
            has_link,
            descriptions,
            attachments,
            time: None,
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

    /// 不经缓存，重新获取当前尝试（[`Self::last_attempt`] 可能来自过期的缓存）
    pub async fn fetch_current_attempt(&self) -> anyhow::Result<Option<String>> {
        CourseAssignmentHandle {
            client: self.client.clone(),
            course: self.course.clone(),
            content: self.content.clone(),
        }
        ._get_current_attempt()
        .await
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
        log::info!("content type: {content_type}");

        let filename = path
            .file_name()
            .context("file name not found")?
            .to_string_lossy()
            .to_string();

        let map = self.get_submit_formfields().await?;
        log::trace!("map: {map:#?}");
        anyhow::ensure!(
            map.contains_key("attempt_id"),
            "教学网未提供新的提交入口，该作业可能不允许再次提交或已关闭提交"
        );

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

            log::debug!("response: {rbody}");
            anyhow::bail!("invalid status {}", st);
        }

        Ok(())
    }

    /// Try to parse the deadline string into a NaiveDateTime.
    pub fn deadline(&self) -> Option<chrono::DateTime<chrono::Local>> {
        let naive_dt = parse_deadline(self.data.deadline.as_deref()?)?;
        // `earliest` is None only for times skipped by a DST transition
        chrono::Local.from_local_datetime(&naive_dt).earliest()
    }

    pub fn deadline_raw(&self) -> Option<&str> {
        self.data.deadline.as_deref()
    }

    pub async fn download_attachment(
        &self,
        uri: &str,
        dest: &std::path::Path,
    ) -> anyhow::Result<()> {
        self.client
            .course_attachment_download(uri, dest, true)
            .await
    }
}

#[derive(Debug, Clone)]
pub struct CourseAnnouncementHandle {
    course: Arc<CourseMeta>,
    content: Arc<CourseContentData>,
}

impl CourseAnnouncementHandle {
    pub fn id(&self) -> String {
        let mut hasher = std::hash::DefaultHasher::new();
        self.course.id.hash(&mut hasher);
        self.content.id.hash(&mut hasher);
        let x = hasher.finish();
        format!("{x:x}")
    }

    pub fn title(&self) -> &str {
        &self.content.title
    }

    pub fn time(&self) -> Option<&str> {
        self.content.time.as_deref()
    }

    pub fn descriptions(&self) -> &[String] {
        &self.content.descriptions
    }

    pub fn attachments(&self) -> &[(String, String)] {
        &self.content.attachments
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

/// 解析教学网的截止时间，如 `2026年11月25日 星期三 下午11:59`
fn parse_deadline(d: &str) -> Option<chrono::NaiveDateTime> {
    let re =
        regex::Regex::new(r"(\d{4})年(\d{1,2})月(\d{1,2})日 星期. (上午|下午)(\d{1,2}):(\d{1,2})")
            .unwrap();

    let caps = re.captures(d)?;
    let year: i32 = caps[1].parse().ok()?;
    let month: u32 = caps[2].parse().ok()?;
    let day: u32 = caps[3].parse().ok()?;
    let mut hour: u32 = caps[5].parse().ok()?;
    let minute: u32 = caps[6].parse().ok()?;

    // 12-hour clock: 上午12 is midnight, 下午1-11 are 13-23
    match (&caps[4], hour) {
        ("上午", 12) => hour = 0,
        ("下午", h) if h < 12 => hour += 12,
        _ => {}
    }

    Some(chrono::NaiveDateTime::new(
        chrono::NaiveDate::from_ymd_opt(year, month, day)?,
        chrono::NaiveTime::from_hms_opt(hour, minute, 0)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_courses_skips_links_without_course_key() {
        // #51：课程栏目里出现不是课程的链接时，不应让整个课程列表失败
        let html = r#"
            <div class="portlet">
              <span class="moduleTitle">当前学期课程</span>
              <ul class="courseListing">
                <li><a href="x?key=_80052_1, y">课程甲</a></li>
                <li><a href="/webapps/blackboard/execute/announcement">课程公告</a></li>
                <li><a>无链接</a></li>
              </ul>
            </div>
            <div class="portlet">
              <span class="moduleTitle">历史课程</span>
              <ul class="courseListing">
                <li><a href="x?key=_70001_1, y">课程乙</a></li>
              </ul>
            </div>
            <div class="portlet"><span class="moduleTitle">我的组织</span></div>
            <div class="portlet"></div>
        "#;
        let dom = scraper::Html::parse_document(html);

        let courses = parse_courses(&dom).unwrap();

        assert_eq!(
            courses,
            vec![
                ("_80052_1".to_owned(), "课程甲".to_owned(), true),
                ("_70001_1".to_owned(), "课程乙".to_owned(), false),
            ]
        );
    }

    #[test]
    fn parse_courses_fails_when_no_course_found() {
        let dom = scraper::Html::parse_document(r#"<div class="portlet"></div>"#);
        assert!(parse_courses(&dom).is_err());
    }

    fn content_li(alt: &str, id: &str, href: &str, details: &str) -> String {
        format!(
            r#"<li><img alt="{alt}"><div class="item" id="{id}"><h3><a href="{href}">{id}</a></h3></div><div class="details">{details}</div></li>"#
        )
    }

    #[test]
    fn parse_content_list_ignores_lists_pasted_into_descriptions() {
        // #43：老师把教学网页面的 HTML 粘进了条目描述，描述里出现嵌套的
        // `#content_listContainer`，其中的 `li`（实测多为空，也可能是完整的条目）
        // 都不是本页的内容条目
        let nested = format!(
            r#"<div class="vtbegenerated"><ul id="content_listContainer"><li id="contentListItem:_8_1"></li>{}</ul></div>"#,
            content_li("文件", "_9_1", "/bbcswebdav/pid-9/xid-9", "")
        );
        let html = format!(
            r#"<ul class="contentList" id="content_listContainer">{}{}</ul>"#,
            content_li("文件", "_1_1", "/bbcswebdav/pid-1/xid-1", &nested),
            content_li(
                "作业",
                "_2_1",
                "/webapps/assignment/uploadAssignment?content_id=_2_1",
                ""
            ),
        );
        let dom = scraper::Html::parse_document(&html);
        let ids: Vec<_> = parse_content_list(&dom).into_iter().map(|c| c.id).collect();
        assert_eq!(ids, ["_1_1", "_2_1"]);
    }

    #[test]
    fn parse_content_list_follows_only_folder_links() {
        // #43：文件、作业、Web 链接等条目的标题也是链接，但它们不是文件夹，
        // 用它们的 id 去请求 listContent.jsp 只会拿回所在文件夹的同一页
        let html = format!(
            r#"<ul class="contentList" id="content_listContainer">{}{}{}{}</ul>"#,
            content_li(
                "内容文件夹",
                "_1_1",
                "/webapps/blackboard/content/listContent.jsp?content_id=_1_1",
                ""
            ),
            content_li("文件", "_2_1", "/bbcswebdav/pid-2/xid-2", ""),
            content_li(
                "作业",
                "_3_1",
                "/webapps/assignment/uploadAssignment?content_id=_3_1",
                ""
            ),
            content_li(
                "Web 链接",
                "_4_1",
                "/webapps/bb-streammedia-hqy-bb_bb60/previewResource.action",
                ""
            ),
        );
        let dom = scraper::Html::parse_document(&html);
        let followed: Vec<_> = parse_content_list(&dom)
            .into_iter()
            .filter(|c| c.has_link)
            .map(|c| c.id)
            .collect();
        assert_eq!(followed, ["_1_1"]);
    }

    #[test]
    fn parse_content_list_recognizes_web_link_and_image() {
        let html = format!(
            r#"<ul class="contentList" id="content_listContainer">{}{}</ul>"#,
            content_li("Web 链接", "_1_1", "https://example.com/", ""),
            content_li("图像", "_2_1", "/bbcswebdav/pid-2/xid-2", ""),
        );
        let dom = scraper::Html::parse_document(&html);
        let kinds: Vec<_> = parse_content_list(&dom)
            .into_iter()
            .map(|c| c.kind)
            .collect();
        assert!(matches!(
            kinds[..],
            [CourseContentKind::WebLink, CourseContentKind::Image]
        ));
    }

    #[test]
    fn test_get_mime_type() {
        assert_eq!(get_mime_type("html"), "text/html");
        assert_eq!(get_mime_type("png"), "image/png");
        assert_eq!(get_mime_type("mp3"), "audio/mpeg");
        assert_eq!(get_mime_type("unknown"), "application/octet-stream");
    }

    #[test]
    fn test_announcement_dedup_key_empty_content_not_collapsed() {
        let k1 = announcement_dedup_key("标题 A", "", "2026-04-04");
        let k2 = announcement_dedup_key("标题 B", "", "2026-04-04");

        assert_ne!(k1, k2);
    }

    #[test]
    fn test_announcement_dedup_key_whitespace_insensitive() {
        let k1 = announcement_dedup_key("标题 A", "正文 内容", "发布时间 10:00");
        let k2 = announcement_dedup_key("标题A", "正文内容", "发布时间10:00");

        assert_eq!(k1, k2);
    }

    #[test]
    fn parses_twelve_hour_deadlines() {
        let at = |d: &str| parse_deadline(d).map(|t| t.format("%Y-%m-%d %H:%M").to_string());
        assert_eq!(
            at("2026年11月25日 星期三 上午9:05").as_deref(),
            Some("2026-11-25 09:05")
        );
        assert_eq!(
            at("2026年11月25日 星期三 上午12:00").as_deref(),
            Some("2026-11-25 00:00")
        );
        assert_eq!(
            at("2026年11月25日 星期三 下午12:30").as_deref(),
            Some("2026-11-25 12:30")
        );
        assert_eq!(
            at("2026年11月25日 星期三 下午11:59").as_deref(),
            Some("2026-11-25 23:59")
        );
        assert_eq!(at("无截止日期"), None);
    }
}
