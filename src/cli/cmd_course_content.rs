use anyhow::Context;

use super::*;

#[derive(clap::Args)]
pub struct CommandCourseContent {
    /// 强制刷新
    #[arg(short, long, default_value = "false")]
    force: bool,

    /// 手机令牌码。当需要使用 OTP 登录，但未提供此参数时，将会从命令行交互式读取 OTP 码。
    #[arg(long, default_value = "")]
    otp_code: String,

    #[command(subcommand)]
    command: CourseContentCommands,
}

#[derive(Subcommand)]
enum CourseContentCommands {
    /// 查看课程列表
    #[command(visible_alias("ls"))]
    List(ListOptions),
}

#[derive(clap::Args)]
pub struct ListOptions {
    /// 显示所有学期的作业（包括已完成的）
    #[arg(long, default_value = "false")]
    all_term: bool,
    /// 指定课程标题
    #[arg(long)]
    course_title: Option<String>,
}

pub async fn run(cmd: CommandCourseContent, m: &MultiProgress) -> anyhow::Result<()> {
    match cmd.command {
        CourseContentCommands::List(opts) => list(m, cmd.force, cmd.otp_code, opts).await?,
    }
    Ok(())
}

async fn get_contents(
    c: &Course,
    pb: indicatif::ProgressBar,
) -> anyhow::Result<Vec<CourseContent>> {
    let fut = async {
        let mut s = c.content_stream();

        // let pb = pbar::new(s.len() as u64).with_message("search contents");
        pb.set_length(s.len() as u64);
        pb.tick();

        let mut contents = Vec::new();
        while let Some(batch) = s.next_batch().await {
            contents.extend(batch);

            pb.set_length(s.len() as u64);
            pb.set_position(s.num_finished() as u64);
            pb.tick();
        }

        pb.finish_with_message("done.");
        Ok(contents)
    };

    let data = utils::with_cache(
        &format!("course_contents_of_{}", c.meta().id()),
        c.client().cache_ttl(),
        fut,
    )
    .await?;

    Ok(data.into_iter().map(|data| c.build_content(data)).collect())
}

async fn get_courses_contents(
    m: &MultiProgress,
    force: bool,
    otp_code: String,
    all_term: bool,
    course_title: Option<&str>,
) -> anyhow::Result<Vec<(Course, Vec<CourseContent>)>> {
    let courses = load_courses(force, !all_term, otp_code).await?;

    let courses = if let Some(course_title) = course_title {
        log::debug!("filtering courses by title: {course_title}");
        courses
            .into_iter()
            .filter(|c| c.long_title().contains(course_title))
            .collect()
    } else {
        courses
    };

    // fetch each course concurrently
    let pb = m.add(pbar::new(courses.len() as u64)).with_prefix("All");
    let futs = courses.into_iter().map(async |c| -> anyhow::Result<_> {
        let c = c.get().await.context("fetch course")?;
        let _pb = m.add(pbar::new(0).with_prefix(c.meta().name().to_owned()));
        let contents = get_contents(&c, _pb)
            .await
            .with_context(|| format!("fetch assignment handles of {}", c.meta().title()))?;

        pb.inc(1);
        Ok((c, contents))
    });
    let courses = try_join_all(futs).await?;
    pb.finish_and_clear();
    m.remove(&pb);

    Ok(courses)
}

pub async fn list(
    m: &MultiProgress,
    force: bool,
    otp_code: String,
    opts: ListOptions,
) -> anyhow::Result<()> {
    let courses = get_courses_contents(
        m,
        force,
        otp_code,
        opts.all_term,
        opts.course_title.as_deref(),
    )
    .await?;

    let mut outbuf = Vec::new();
    for (c, contents) in courses {
        writeln!(outbuf, "{BL}{B}{}{B:#}{BL:#}", c.meta().title())?;

        for ct in &contents {
            write!(
                outbuf,
                "{D}•{D:#} {MG}({:?}){MG:#} {}",
                ct.kind(),
                ct.title(),
            )?;
            if !ct.attachments().is_empty() {
                write!(outbuf, " [{B}{}{B:#} 附件]", ct.attachments().len())?;
            }
            writeln!(outbuf, " {D}{}{D:#}", ct.ccid())?;
        }
    }

    // write to stdout
    buf_try!(@try fs::stdout().write_all(outbuf).await);

    Ok(())
}
