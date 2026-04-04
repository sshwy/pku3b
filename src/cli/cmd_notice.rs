use std::sync::Arc;

use anyhow::Context;

use super::*;

#[derive(clap::Args)]
pub struct CommandNotice {
    /// 强制刷新
    #[arg(short, long, default_value = "false")]
    force: bool,

    #[command(subcommand)]
    command: NoticeCommands,
}

#[derive(Subcommand)]
enum NoticeCommands {
    /// 查看课程公告/通知列表
    #[command(visible_alias("ls"))]
    List {
        /// 显示所有学期的课程公告
        #[arg(long, default_value = "false")]
        all_term: bool,
    },
    /// 按 ID 查看公告详情
    Show {
        /// 公告 ID（可通过 `pku3b notice ls` 查看）
        id: String,
        /// 在所有学期的课程公告范围中查找
        #[arg(long, default_value = "false")]
        all_term: bool,
    },
}

pub async fn run(cmd: CommandNotice) -> anyhow::Result<()> {
    match cmd.command {
        NoticeCommands::List { all_term } => list(cmd.force, !all_term).await?,
        NoticeCommands::Show { id, all_term } => show(cmd.force, !all_term, &id).await?,
    }
    Ok(())
}

type AnnouncementListItem = (Arc<Course>, String, CourseAnnouncementHandle);

async fn get_announcements(
    course: &Course,
    pb: indicatif::ProgressBar,
) -> anyhow::Result<Vec<CourseAnnouncementHandle>> {
    let announcements = course
        .list_announcements_from_coursepage()
        .await
        .with_context(|| {
            format!(
                "fetch announcements from course page for {}",
                course.meta().title()
            )
        })?;
    pb.finish_with_message("done.");
    Ok(announcements)
}

async fn get_courses_and_announcements(
    force: bool,
    cur_term: bool,
) -> anyhow::Result<Vec<(Course, Vec<CourseAnnouncementHandle>)>> {
    let courses = load_courses(force, cur_term).await?;

    let m = indicatif::MultiProgress::new();
    let pb = m.add(pbar::new(courses.len() as u64)).with_prefix("All");
    let futs = courses
        .into_iter()
        .map(async |course| -> anyhow::Result<_> {
            let course = course.get().await.context("fetch course")?;
            let announcements = get_announcements(
                &course,
                m.add(pbar::new(0).with_prefix(course.meta().name().to_owned())),
            )
            .await
            .with_context(|| format!("fetch announcement handles of {}", course.meta().title()))?;

            pb.inc_length(announcements.len() as u64);
            let futs = announcements
                .into_iter()
                .map(async |announcement| -> anyhow::Result<_> {
                    pb.inc(1);
                    Ok(announcement)
                });
            let announcements = try_join_all(futs).await?;

            pb.inc(1);
            Ok((course, announcements))
        });
    let courses = try_join_all(futs).await?;
    pb.finish_and_clear();
    m.clear().unwrap();
    drop(pb);
    drop(m);

    Ok(courses)
}

pub async fn list(force: bool, cur_term: bool) -> anyhow::Result<()> {
    let courses = get_courses_and_announcements(force, cur_term).await?;
    let all_announcements = courses
        .iter()
        .flat_map(|(course, announcements)| {
            announcements.iter().map(move |announcement| {
                (course.to_owned(), announcement.id(), announcement.clone())
            })
        })
        .collect::<Vec<_>>();

    let announcements = sort_announcements_owned(all_announcements);
    list_brief(announcements).await
}

async fn list_brief(items: Vec<(Course, String, CourseAnnouncementHandle)>) -> anyhow::Result<()> {
    let mut outbuf = Vec::new();
    let title = "课程公告/通知";
    let total = items.len();
    writeln!(outbuf, "{D}>{D:#} {B}{title} ({total}){B:#} {D}<{D:#}\n")?;

    for (idx, (course, id, announcement)) in items.iter().enumerate() {
        write!(outbuf, "{GR}[{:>2}]{GR:#} ", idx + 1)?;
        write!(
            outbuf,
            "{BL}{B}{}{B:#}{BL:#} {D}>{D:#} {}",
            course.meta().name(),
            announcement.title()
        )?;
        let att_count = announcement.attachments().len();
        if att_count > 0 {
            write!(outbuf, " ({GR}{att_count} 个附件{GR:#})")?;
        }
        writeln!(outbuf, " {D}{id}{D:#}")?;
    }

    buf_try!(@try fs::stdout().write_all(outbuf).await);
    Ok(())
}

pub async fn show(force: bool, cur_term: bool, id: &str) -> anyhow::Result<()> {
    let items = fetch_announcements(force, cur_term).await?;
    let Some((course, ann_id, announcement)) =
        items.into_iter().find(|(_, ann_id, _)| ann_id == id)
    else {
        anyhow::bail!("announcement with id {} not found", id);
    };

    let mut outbuf = Vec::new();
    writeln!(outbuf, "{D}>{D:#} {B}公告详情{B:#} {D}<{D:#}\n")?;
    write_announcement_detail(&mut outbuf, &ann_id, &course, &announcement).context("io error")?;
    buf_try!(@try fs::stdout().write_all(outbuf).await);
    Ok(())
}

fn sort_announcements_owned(
    mut items: Vec<(Course, String, CourseAnnouncementHandle)>,
) -> Vec<(Course, String, CourseAnnouncementHandle)> {
    items.sort_by(|a, b| match (b.2.time(), a.2.time()) {
        (Some(time_b), Some(time_a)) => time_b.cmp(time_a),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    items
}

fn sort_announcements_items(mut items: Vec<AnnouncementListItem>) -> Vec<AnnouncementListItem> {
    items.sort_by(|a, b| match (b.2.time(), a.2.time()) {
        (Some(time_b), Some(time_a)) => time_b.cmp(time_a),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    items
}

fn write_announcement_detail(
    buf: &mut Vec<u8>,
    id: &str,
    course: &Course,
    announcement: &CourseAnnouncementHandle,
) -> std::io::Result<()> {
    writeln!(
        buf,
        "{BL}{B}{}{B:#}{BL:#} {D}>{D:#} {BL}{B}{}{B:#}{BL:#}",
        course.meta().name(),
        announcement.title()
    )?;
    writeln!(buf, "{D}ID:{D:#} {id}")?;

    if let Some(time) = announcement.time() {
        writeln!(buf, "{D}发布时间:{D:#} {time}")?;
    }

    if !announcement.descriptions().is_empty() {
        writeln!(buf)?;
        for line in announcement.descriptions() {
            writeln!(buf, "{line}")?;
        }
    }

    if !announcement.attachments().is_empty() {
        writeln!(buf)?;
        for (name, _) in announcement.attachments() {
            writeln!(buf, "{D}[附件]{D:#} {UL}{name}{UL:#}")?;
        }
    }

    writeln!(buf)?;
    Ok(())
}

async fn fetch_announcements(
    force: bool,
    cur_term: bool,
) -> anyhow::Result<Vec<AnnouncementListItem>> {
    let courses = get_courses_and_announcements(force, cur_term).await?;

    let mut all_announcements = courses
        .into_iter()
        .flat_map(|(course, announcements)| {
            let course = Arc::new(course);
            announcements
                .into_iter()
                .map(move |announcement| (course.clone(), announcement.id(), announcement))
        })
        .collect::<Vec<_>>();

    all_announcements = sort_announcements_items(all_announcements);
    Ok(all_announcements)
}
