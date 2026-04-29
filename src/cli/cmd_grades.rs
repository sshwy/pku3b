use anyhow::Context;
use serde::Deserialize;

use super::*;

#[derive(clap::Args)]
pub struct CommandGrades {
    /// 强制刷新
    #[arg(short, long, default_value = "false")]
    force: bool,

    /// 显示所有学期的成绩
    #[arg(long, default_value = "false")]
    all_term: bool,

    /// 手机令牌码
    #[arg(long, default_value = "")]
    otp_code: String,
}

#[derive(Debug, Deserialize)]
struct UserInfo {
    id: String,
}

#[derive(Debug, Deserialize)]
struct CourseEnrollment {
    #[serde(rename = "courseId")]
    course_id: String,
}

#[derive(Debug, Deserialize)]
struct CourseDetail {
    name: String,
    availability: Option<Availability>,
}

#[derive(Debug, Deserialize)]
struct Availability {
    available: String,
}

#[derive(Debug, Deserialize)]
struct GradebookColumns {
    results: Vec<GradebookColumn>,
}

#[derive(Debug, Deserialize)]
struct GradebookColumn {
    id: String,
    name: String,
    score: Option<ColumnScore>,
    grading: Option<Grading>,
}

#[derive(Debug, Deserialize)]
struct ColumnScore {
    possible: f64,
}

#[derive(Debug, Deserialize)]
struct Grading {
    #[serde(rename = "type")]
    grading_type: String,
}

#[derive(Debug, Deserialize)]
struct GradeUsers {
    results: Vec<GradeUser>,
}

#[derive(Debug, Deserialize)]
struct GradeUser {
    #[serde(rename = "displayGrade")]
    display_grade: Option<DisplayGrade>,
}

#[derive(Debug, Deserialize)]
struct DisplayGrade {
    score: Option<f64>,
}

#[derive(Debug)]
struct GradeRecord {
    course_name: String,
    column_name: String,
    score: Option<f64>,
    possible: f64,
}

pub async fn run(cmd: CommandGrades) -> anyhow::Result<()> {
    let (client, _, sp) = load_client_courses(cmd.force, !cmd.all_term, cmd.otp_code).await?;

    sp.set_message("fetching user info...");

    // 获取用户信息
    let user_info: UserInfo = client
        .api_get("https://course.pku.edu.cn/learn/api/public/v1/users/me")
        .await
        .context("fetch user info")?;

    sp.set_message("fetching courses...");

    // 获取课程列表 (返回格式: {"results": [...]})
    let courses_resp: serde_json::Value = client
        .api_get(&format!(
            "https://course.pku.edu.cn/learn/api/public/v1/users/{}/courses",
            user_info.id
        ))
        .await
        .context("fetch user courses")?;

    let enrollments: Vec<CourseEnrollment> =
        serde_json::from_value(courses_resp["results"].clone())
            .context("parse courses")?;

    let mut all_grades = Vec::new();
    let total = enrollments.len();

    for (i, enrollment) in enrollments.iter().enumerate() {
        sp.set_message(format!("fetching grades {}/{}...", i + 1, total));

        // 获取课程详情
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

        // 跳过不可用的课程
        if detail
            .availability
            .as_ref()
            .map(|a| a.available != "Yes")
            .unwrap_or(true)
        {
            continue;
        }

        // 获取成绩列
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
            // 跳过计算列（加权总计、总计）
            if let Some(grading) = &col.grading {
                if grading.grading_type == "Calculated"
                    && (col.name.contains("总计") && !col.name.contains("平时"))
                {
                    continue;
                }
            }

            // 获取成绩
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
                score,
                possible,
            });
        }
    }

    sp.finish_with_message("done.");

    // 输出成绩表格
    print_grades_table(&all_grades);

    Ok(())
}

fn print_grades_table(grades: &[GradeRecord]) {
    if grades.is_empty() {
        println!("暂无成绩数据");
        return;
    }

    // 按课程分组
    let mut courses: Vec<String> = Vec::new();
    let mut course_grades: std::collections::HashMap<String, Vec<&GradeRecord>> =
        std::collections::HashMap::new();

    for g in grades {
        if !course_grades.contains_key(&g.course_name) {
            courses.push(g.course_name.clone());
        }
        course_grades
            .entry(g.course_name.clone())
            .or_default()
            .push(g);
    }

    println!("{}", "=".repeat(70));
    println!("  北京大学教学网 成绩查询");
    println!("{}", "=".repeat(70));

    for course_name in &courses {
        let items = &course_grades[course_name];
        println!();
        println!("  {}", course_name);
        println!("{}", "-".repeat(70));
        println!("  {:<40} {:>10} {:>10}", "考核项", "得分", "满分");
        println!("{}", "-".repeat(70));

        for item in items {
            let score_str = match item.score {
                Some(s) => format!("{:.1}", s),
                None => "--".to_string(),
            };
            let possible_str = if item.possible > 0.0 {
                format!("{:.0}", item.possible)
            } else {
                "--".to_string()
            };
            println!(
                "  {:<40} {:>10} {:>10}",
                item.column_name, score_str, possible_str
            );
        }
    }

    println!();
    println!("{}", "=".repeat(70));
}
