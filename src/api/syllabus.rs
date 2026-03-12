use super::*;

impl Client {
    pub async fn syllabus(
        &self,
        username: &str,
        password: &str,
        dual: Option<DualDegree>,
    ) -> anyhow::Result<Syllabus> {
        let c = &self.0.http_client;

        if let Some(dual) = dual {
            let sttp = if matches!(dual, DualDegree::Major) {
                "bzx"
            } else {
                "bfx"
            };
            c.sb_login_dual_degree(username, password, sttp)
                .await
                .context("syllabus login dual degree")?;
        } else {
            c.sb_login(username, password)
                .await
                .context("syllabus login")?;
        }

        Ok(Syllabus {
            client: self.clone(),
            username: username.to_owned(),
        })
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
                    "课程号",
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
                name: row_values[1].to_owned(),
                category: row_values[2].to_owned(),
                score: row_values[3].to_owned(),
                hours_per_week: row_values[4].to_owned(),
                teacher: row_values[5].to_owned(),
                class_id: row_values[6].to_owned(),
                department: row_values[7].to_owned(),
                classroom: row_values[8].to_owned(),
                custom_n_or_np: row_values[9].to_owned(),
                status: row_values[10].to_owned(),
            });
        }
        Ok(r)
    }

    /// 获取补选总页数和已选上课程，必须在获取补选课程前调用，否则会返回空页面
    pub async fn get_supplements_total_and_elected(
        &self,
    ) -> anyhow::Result<(usize, Vec<SyllabusBaseCourseData>)> {
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

        let table_sel = Selector::parse("table.datagrid").unwrap();
        let table = dom.select(&table_sel).nth(1).context("table not found")?;
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
                    "课程号",
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
                    "限数/已选",
                    "选课状态",
                    "退选",
                ],
            "unexpected column names: {:?}",
            col_names
        );

        let mut r = Vec::new();
        for row in rows {
            // 对应 "Page 1 of 1  First / Previous   Next / Last" 这一行
            if row.child_elements().count() <= 3 {
                continue;
            }
            let row_values = row
                .child_elements()
                .map(|el| el.text().collect::<String>().trim().to_owned())
                .collect::<Vec<_>>();
            r.push(SyllabusBaseCourseData {
                name: row_values[1].to_owned(),
                category: row_values[2].to_owned(),
                score: row_values[3].to_owned(),
                hours_per_week: row_values[4].to_owned(),
                teacher: row_values[5].to_owned(),
                class_id: row_values[6].to_owned(),
                department: row_values[7].to_owned(),
                classroom: row_values[9].to_owned(),
                custom_n_or_np: row_values[10].to_owned(),
                status: row_values[11].to_owned(),
            });
        }

        Ok((total, r))
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
                    "课程号",
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
                ]
                || col_names
                    == [
                        "课程号",
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
                        "限数/已选",
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
                    name: row_values[1].to_owned(),
                    category: row_values[2].to_owned(),
                    score: row_values[3].to_owned(),
                    hours_per_week: row_values[4].to_owned(),
                    teacher: row_values[5].to_owned(),
                    class_id: row_values[6].to_owned(),
                    department: row_values[7].to_owned(),
                    classroom: row_values[9].to_owned(),
                    custom_n_or_np: row_values[10].to_owned(),
                    status: row_values[11].to_owned(),
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
    #[cfg(feature = "autoelect")]
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
            log::debug!("captcha code recognition: {code}");

            let r = self
                .client
                .sb_send_validation(&self.username, &code)
                .await?;
            log::trace!("captcha validation response: {r}");
            if r == 2 {
                break;
            }
            log::warn!("验证码不正确，正在重试...");
            compio::time::sleep(std::time::Duration::from_secs(1)).await;
        }

        match self
            .client
            .sb_elect_by_url(&format!(
                "https://elective.pku.edu.cn{}",
                course.supplement_url
            ))
            .await
        {
            Ok(dom) => {
                let td_sel = Selector::parse("td#msgTips").unwrap();
                if let Some(td) = dom.select(&td_sel).nth(0) {
                    let text = td.text().collect::<String>().trim().to_owned();
                    println!("{}: {}", course.name, text);
                } else {
                    log::warn!("{} 选择完成，没有找到提示信息", course.name);
                }
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
    #[cfg(feature = "autoelect")]
    pub fn is_full(&self) -> anyhow::Result<bool> {
        status_is_full(&self.status)
    }
}

#[cfg(feature = "autoelect")]
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
#[allow(unused)]
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

#[derive(Debug, Clone)]
pub enum DualDegree {
    /// 主修
    Major,
    /// 辅双
    Minor,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(feature = "autoelect")]
    fn test_status_is_full() {
        assert!(!status_is_full("30 /25 ").unwrap());
        assert!(status_is_full(" 30/ 30").unwrap());
        assert!(status_is_full("30 / 35 ").unwrap());
        assert!(status_is_full("invalid").is_err());
    }
}
