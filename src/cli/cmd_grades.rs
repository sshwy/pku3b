use anyhow::Context;
use serde::Deserialize;

use super::*;

#[derive(clap::Args)]
pub struct CommandGrades {
    #[arg(short, long, default_value = "false")]
    force: bool,
    #[arg(long, default_value = "false")]
    all_term: bool,
    #[arg(long, default_value = "")]
    otp_code: String,
}

#[derive(Debug, Deserialize)]
struct UserInfo { id: String }

#[derive(Debug, Deserialize)]
struct CourseEnrollment { #[serde(rename = "courseId")] course_id: String }

#[derive(Debug, Deserialize)]
struct CourseDetail { name: String, availability: Option<Availability> }

#[derive(Debug, Deserialize)]
struct Availability { available: String }

#[derive(Debug, Deserialize)]
struct GradebookColumns { results: Vec<GradebookColumn> }

#[derive(Debug, Deserialize)]
struct GradebookColumn {
    id: String, name: String,
    score: Option<ColumnScore>, grading: Option<Grading>,
}

#[derive(Debug, Deserialize)]
struct ColumnScore { possible: f64 }

#[derive(Debug, Deserialize)]
struct Grading { #[serde(rename = "type")] grading_type: String }

#[derive(Debug, Deserialize)]
struct GradeUsers { results: Vec<GradeUser> }

#[derive(Debug, Deserialize)]
struct GradeUser { #[serde(rename = "displayGrade")] display_grade: Option<DisplayGrade> }

#[derive(Debug, Deserialize)]
struct DisplayGrade { score: Option<f64> }

#[derive(Debug)]
struct GradeRecord {
    course_name: String, column_name: String,
    score: Option<f64>, possible: f64,
}

fn extract_term(name: &str) -> &str {
    name.rfind('(').map(|i| &name[i..]).unwrap_or("")
}

pub async fn run(cmd: CommandGrades) -> anyhow::Result<()> {
    let (client, _, sp) = load_client_courses(cmd.force, !cmd.all_term, cmd.otp_code).await?;

    sp.set_message("fetching user info...");
    let user_info: UserInfo = client
        .api_get("https://course.pku.edu.cn/learn/api/public/v1/users/me")
        .await
        .context("fetch user info")?;

    sp.set_message("fetching courses...");
    let courses_resp: serde_json::Value = client
        .api_get(&format!(
            "https://course.pku.edu.cn/learn/api/public/v1/users/{}/courses",
            user_info.id
        ))
        .await
        .context("fetch user courses")?;

    let enrollments: Vec<CourseEnrollment> =
        serde_json::from_value(courses_resp["results"].clone()).context("parse courses")?;

    let mut all_grades = Vec::new();
    let total = enrollments.len();

    for (i, enrollment) in enrollments.iter().enumerate() {
        sp.set_message(format!("fetching grades {}/{}...", i + 1, total));

        let detail: CourseDetail = match client
            .api_get(&format!(
                "https://course.pku.edu.cn/learn/api/public/v1/courses/{}",
                enrollment.course_id
            ))
            .await
        {
            Ok(d) => d,
            Err(_) => continue,
        };

        if detail.availability.as_ref().map(|a| a.available != "Yes").unwrap_or(true) {
            continue;
        }

        let columns: GradebookColumns = match client
            .api_get(&format!(
                "https://course.pku.edu.cn/learn/api/public/v2/courses/{}/gradebook/columns",
                enrollment.course_id
            ))
            .await
        {
            Ok(c) => c,
            Err(_) => continue,
        };

        for col in &columns.results {
            if let Some(grading) = &col.grading {
                if grading.grading_type == "Calculated"
                    && (col.name.contains("总计") && !col.name.contains("平时"))
                {
                    continue;
                }
            }

            let grade_data: Option<GradeUser> = match client
                .api_get::<GradeUsers>(&format!(
                    "https://course.pku.edu.cn/learn/api/public/v2/courses/{}/gradebook/columns/{}/users",
                    enrollment.course_id, col.id
                ))
                .await
            {
                Ok(data) => data.results.into_iter().next(),
                Err(_) => None,
            };

            let possible = col.score.as_ref().map(|s| s.possible).unwrap_or(0.0);
            let score = grade_data.and_then(|g| g.display_grade).and_then(|d| d.score);

            all_grades.push(GradeRecord {
                course_name: detail.name.clone(),
                column_name: col.name.clone(),
                score, possible,
            });
        }
    }

    sp.finish_with_message("done.");
    print_grades(&all_grades, cmd.all_term).await?;
    Ok(())
}

async fn print_grades(grades: &[GradeRecord], all_term: bool) -> anyhow::Result<()> {
    if grades.is_empty() {
        println!("暂无成绩数据");
        return Ok(());
    }

    let latest_term = grades.iter().map(|g| extract_term(&g.course_name)).max().unwrap_or("").to_string();

    let mut courses: Vec<&str> = Vec::new();
    let mut course_map: std::collections::HashMap<&str, Vec<&GradeRecord>> =
        std::collections::HashMap::new();
    for g in grades {
        if !all_term && extract_term(&g.course_name) != latest_term {
            continue;
        }
        if !course_map.contains_key(g.course_name.as_str()) {
            courses.push(&g.course_name);
        }
        course_map.entry(g.course_name.as_str()).or_default().push(g);
    }

    let mut outbuf = Vec::new();
    let mut displayed = 0usize;

    for course_name in &courses {
        let items = &course_map[*course_name];
        if items.iter().all(|item| item.score.is_none()) {
            continue;
        }
        if displayed == 0 {
            writeln!(outbuf, "{D}>{D:#} {B}成绩查询{B:#} {D}<{D:#}\n")?;
        }
        displayed += 1;
        writeln!(outbuf, "{BL}{B}{course_name}{B:#}{BL:#}")?;
        for item in items {
            if item.score.is_none() { continue; }
            write!(outbuf, "{D}*{D:#} {} ", item.column_name)?;
            match item.score {
                Some(s) => write!(outbuf, "{GR}{:.1}{GR:#}", s)?,
                None => write!(outbuf, "{D}--{D:#}")?,
            };
            if item.possible > 0.0 {
                write!(outbuf, "{D} / {:.0}{D:#}", item.possible)?;
            }
            writeln!(outbuf)?;
        }
        writeln!(outbuf)?;
    }

    if displayed == 0 {
        println!("暂无成绩数据");
    } else {
        buf_try!(@try fs::stdout().write_all(outbuf).await);
    }
    Ok(())
}
