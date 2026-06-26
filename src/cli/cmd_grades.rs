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

fn extract_term(name: &str) -> &str {
    name.rfind('(').map(|i| &name[i..]).unwrap_or("")
}

pub async fn run(cmd: CommandGrades, ctx: &CommandCtx<'_>) -> anyhow::Result<()> {
    let (b, sp) = load_blackboard(ctx, !cmd.force, cmd.otp_code, cmd.force).await?;

    sp.set_message("fetching user info...");
    let user_id = b.user_info_id().await?;

    sp.set_message("fetching courses...");
    let mut enrollments = b.user_courses(&user_id).await?;
    enrollments.retain(|e| e.course_role_id == "Student");

    let mut all_grades = Vec::new();
    let total = enrollments.len();

    for (i, enrollment) in enrollments.iter().enumerate() {
        sp.set_message(format!("fetching grades {}/{}...", i + 1, total));

        let detail = match b.course_detail(&enrollment.course_id).await {
            Ok(d) => d,
            Err(e) => {
                log::error!("error fetching course detail: {e}");
                continue;
            }
        };

        if !detail.data().is_available() {
            continue;
        }

        all_grades.extend(detail.all_grades().await?);
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

    let latest_term = grades
        .iter()
        .map(|g| extract_term(&g.course_name))
        .max()
        .unwrap_or("")
        .to_string();

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
        course_map
            .entry(g.course_name.as_str())
            .or_default()
            .push(g);
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
            if item.score.is_none() {
                continue;
            }
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
