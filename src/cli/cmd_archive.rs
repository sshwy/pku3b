use anyhow::Context;

use super::*;

#[derive(clap::Args)]
pub struct CommandArchive {
    /// 课程标题关键字或课程 ID；匹配多门课程时会交互选择
    course: String,

    /// 归档输出目录
    #[arg(short = 'o', long)]
    outdir: std::path::PathBuf,

    /// 归档板块，可用 all 或逗号分隔的 assignment,course-content,video
    #[arg(long, default_value = "all", value_parser = parse_archive_sections)]
    sections: ArchiveSections,

    /// 在所有学期的课程范围中查找
    #[arg(long, default_value = "false")]
    all_term: bool,

    /// 覆盖已存在文件；默认跳过已存在文件
    #[arg(long, default_value = "false")]
    overwrite: bool,

    /// 强制刷新
    #[arg(short, long, default_value = "false")]
    force: bool,

    /// 手机令牌码。当需要使用 OTP 登录，但未提供此参数时，将会从命令行交互式读取 OTP 码。
    #[arg(long, default_value = "")]
    otp_code: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ArchiveSections {
    assignment: bool,
    course_content: bool,
    video: bool,
}

impl ArchiveSections {
    fn all() -> Self {
        Self {
            assignment: true,
            course_content: true,
            video: true,
        }
    }

    fn none() -> Self {
        Self {
            assignment: false,
            course_content: false,
            video: false,
        }
    }

    fn is_empty(self) -> bool {
        !self.assignment && !self.course_content && !self.video
    }
}

pub async fn run(cmd: CommandArchive, ctx: &CommandCtx<'_>) -> anyhow::Result<()> {
    if cmd.sections.video {
        #[cfg(not(feature = "video-download"))]
        anyhow::bail!(
            "`archive --sections video` requires building pku3b with the `video-download` feature"
        );
    }

    let courses = load_courses(ctx, cmd.force, !cmd.all_term, cmd.otp_code).await?;
    let selected = select_course_handle(courses, &cmd.course, "请选择要归档的课程")?;
    let title_has_duplicates = selected.title_has_duplicates;

    let sp = ctx.spinner();
    sp.set_message("fetching course...");
    let course = selected.handle.get().await.context("fetch course")?;
    ctx.remove_spinner(sp);

    let course_dir = course_archive_dir(&cmd.outdir, &course, title_has_duplicates);
    fs::create_dir_all(&course_dir)
        .await
        .with_context(|| format!("create output directory {}", course_dir.display()))?;

    println!(
        "开始归档课程 {B}{}{B:#} 到 {}",
        course.meta().long_title(),
        course_dir.display()
    );

    if cmd.sections.assignment {
        let outdir = course_dir.join("assignments");
        cmd_assignment::archive_course_assignments(ctx, &course, &outdir, cmd.overwrite).await?;
    }

    if cmd.sections.course_content {
        let outdir = course_dir.join("course-content");
        cmd_course_content::archive_course_contents(ctx, &course, &outdir, cmd.overwrite).await?;
    }

    if cmd.sections.video {
        #[cfg(feature = "video-download")]
        {
            let outdir = course_dir.join("videos");
            cmd_video::archive_course_videos(ctx, &course, &outdir, cmd.overwrite).await?;
        }
    }

    println!("课程归档完成: {}", course_dir.display());
    Ok(())
}

fn course_archive_dir(
    outdir: &std::path::Path,
    course: &Course,
    title_has_duplicates: bool,
) -> std::path::PathBuf {
    let mut dirname = sanitize_filename_part(course.meta().long_title());
    if title_has_duplicates {
        dirname = format!("{}_{}", dirname, sanitize_filename_part(course.meta().id()));
    }
    outdir.join(dirname)
}

fn parse_archive_sections(input: &str) -> Result<ArchiveSections, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("sections cannot be empty".to_owned());
    }

    let mut sections = ArchiveSections::none();
    for part in input.split(',') {
        let part = part.trim();
        match part {
            "all" => return Ok(ArchiveSections::all()),
            "assignment" | "assignments" | "a" => sections.assignment = true,
            "course-content" | "course_content" | "coursecontent" | "cc" => {
                sections.course_content = true;
            }
            "video" | "videos" | "v" => sections.video = true,
            "" => return Err("sections cannot contain an empty item".to_owned()),
            unknown => {
                return Err(format!(
                    "unknown section '{unknown}', expected all, assignment, course-content, or video"
                ));
            }
        }
    }

    if sections.is_empty() {
        Err("at least one section must be selected".to_owned())
    } else {
        Ok(sections)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sections_all() {
        assert_eq!(
            parse_archive_sections("all").unwrap(),
            ArchiveSections::all()
        );
    }

    #[test]
    fn parse_sections_subset() {
        assert_eq!(
            parse_archive_sections("assignment,course-content").unwrap(),
            ArchiveSections {
                assignment: true,
                course_content: true,
                video: false,
            }
        );
    }

    #[test]
    fn parse_sections_rejects_unknown() {
        assert!(parse_archive_sections("assignment,grades").is_err());
    }
}
