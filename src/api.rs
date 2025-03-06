use anyhow::Context;
use chrono::TimeZone;
use rand::{distr::Open01, prelude::*};
use std::{collections::HashMap, str::FromStr, sync::Arc};

use crate::utils::with_cache;

#[derive(Debug)]
struct ClientInner {
    http_client: cyper::Client,
    cache_ttl: Option<std::time::Duration>,
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
    pub fn new(cache_ttl: Option<std::time::Duration>) -> Self {
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
                Ok(CourseHandle(
                    CourseHandleInner {
                        client: self.client.clone(),
                        key,
                        title,
                    }
                    .into(),
                ))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        Ok(courses)
    }
}

struct CourseHandleInner {
    client: Client,
    key: String,
    title: String,
}

impl std::fmt::Debug for CourseHandleInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CourseHandleInner")
            .field("key", &self.key)
            .field("title", &self.title)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct CourseHandle(Arc<CourseHandleInner>);

impl CourseHandle {
    pub async fn _get(&self) -> anyhow::Result<HashMap<String, String>> {
        let res = self
            .0
            .client
            .get(COURSE_INFO_PAGE)?
            .query(&[
                ("method", "search"),
                ("context", "course_entry"),
                ("course_id", &self.0.key),
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
            &format!("CourseHandle::_get_{}", self.0.key),
            self.0.client.cache_ttl(),
            self._get(),
        )
        .await?;

        Ok(Course {
            handle: self.clone(),
            entries,
        })
    }
}

#[derive(Debug)]
pub struct Course {
    handle: CourseHandle,
    entries: HashMap<String, String>,
}

impl Course {
    pub fn name(&self) -> &str {
        &self.handle.0.title.split(": ").nth(1).unwrap()
    }
    async fn _get_assignments(&self) -> anyhow::Result<Vec<(String, String)>> {
        let Some(uri) = self.entries.get("课程作业") else {
            return Ok(Vec::new());
        };

        let res = self
            .handle
            .0
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
    pub async fn get_assignments(&self) -> anyhow::Result<Vec<CourseAssignmentHandle>> {
        let assignments = with_cache(
            &format!("Course::_get_assignments_{}", self.handle.0.key),
            self.handle.0.client.cache_ttl(),
            self._get_assignments(),
        )
        .await?;

        if std::env::var("PKU3B_DEBUG").is_ok() {
            eprintln!("key: {}", self.handle.0.key);
            dbg!(&assignments);
        }

        let assignments = assignments
            .into_iter()
            // remove assignment answers
            .filter(|(_, url)| !url.starts_with("/bbcswebdav"))
            .map(|(title, uri)| {
                let uri = http::Uri::from_str(&uri).context("parse uri failed")?;
                let qs = uri
                    .query()
                    .context("uri has no query")?
                    .split('&')
                    .collect::<Vec<_>>();
                let course_id = qs
                    .iter()
                    .find(|s| s.starts_with("course_id="))
                    .context("course_id not found")?
                    .strip_prefix("course_id=")
                    .context("course_id not found")?
                    .to_owned();
                let content_id = qs
                    .iter()
                    .find(|s| s.starts_with("content_id="))
                    .context("content_id not found")?
                    .strip_prefix("content_id=")
                    .context("content_id not found")?
                    .to_owned();

                Ok(CourseAssignmentHandle(
                    CourseAssignmentInner {
                        client: self.handle.0.client.clone(),
                        title,
                        course_id,
                        content_id,
                    }
                    .into(),
                ))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        Ok(assignments)
    }
}

pub struct CourseAssignmentInner {
    client: Client,
    title: String,
    course_id: String,
    content_id: String,
}

impl std::fmt::Debug for CourseAssignmentInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CourseAssignmentInner")
            .field("title", &self.title)
            .field("course_id", &self.course_id)
            .field("content_id", &self.content_id)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct CourseAssignmentHandle(Arc<CourseAssignmentInner>);

impl CourseAssignmentHandle {
    async fn _get(&self) -> anyhow::Result<CourseAssignmentDetailData> {
        let res = self
            .0
            .client
            .get("https://course.pku.edu.cn/webapps/assignment/uploadAssignment")?
            .query(&[
                ("action", "newAttempt"),
                ("content_id", &self.0.content_id),
                ("course_id", &self.0.course_id),
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

        Ok(CourseAssignmentDetailData {
            descriptions: desc,
            attachments,
            deadline,
        })
    }
    pub async fn get(&self) -> anyhow::Result<CourseAssignmentDetail> {
        let data = with_cache(
            &format!(
                "CourseAssignmentHandle::_get_{}_{}",
                self.0.content_id, self.0.course_id
            ),
            self.0.client.cache_ttl(),
            self._get(),
        )
        .await?;

        Ok(CourseAssignmentDetail {
            handle: self.clone(),
            data,
        })
    }

    async fn _get_current_attempt(&self) -> anyhow::Result<Option<String>> {
        let res = self
            .0
            .client
            .get("https://course.pku.edu.cn/webapps/assignment/uploadAssignment")?
            .query(&[
                ("mode", "view"),
                ("content_id", &self.0.content_id),
                ("course_id", &self.0.course_id),
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

    pub async fn get_current_attempt(&self) -> anyhow::Result<Option<String>> {
        let attempt_label = with_cache(
            &format!(
                "CourseAssignmentHandle::_get_current_attempt_{}_{}",
                self.0.content_id, self.0.course_id
            ),
            self.0.client.cache_ttl(),
            self._get_current_attempt(),
        )
        .await?;

        Ok(attempt_label)
    }
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct CourseAssignmentDetailData {
    descriptions: Vec<String>,
    attachments: Vec<(String, String)>,
    deadline: String,
}

pub struct CourseAssignmentDetail {
    handle: CourseAssignmentHandle,
    data: CourseAssignmentDetailData,
}

impl CourseAssignmentDetail {
    pub fn title(&self) -> &str {
        &self.handle.0.title
    }

    pub fn descriptions(&self) -> &[String] {
        &self.data.descriptions
    }

    pub fn attachments(&self) -> &[(String, String)] {
        &self.data.attachments
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
