use crate::api;
use crate::cli::{self, pbar, utils};
use anyhow::Context;
use compio::buf::buf_try;
use compio::fs;
use compio::io::AsyncWriteExt;
use futures_util::future::try_join_all;
use std::io::Write as _;
use std::sync::Arc;
use utils::style::*;

type AnnouncementListItem = (Arc<api::Course>, String, api::CourseAnnouncementHandle);

async fn get_announcements(
    c: &api::Course,
    pb: indicatif::ProgressBar,
) -> anyhow::Result<Vec<api::CourseAnnouncementHandle>> {
    // 使用 course page 获取公告，而不是 content stream
    let r = c
        .list_announcements_from_coursepage()
        .await
        .with_context(|| {
            format!(
                "fetch announcements from course page for {}",
                c.meta().title()
            )
        })?;
    pb.finish_with_message("done.");
    Ok(r)
}

async fn get_courses_and_announcements(
    force: bool,
    cur_term: bool,
) -> anyhow::Result<Vec<(api::Course, Vec<api::CourseAnnouncementHandle>)>> {
    let courses = cli::load_courses(force, cur_term).await?;

    // fetch each course concurrently
    let m = indicatif::MultiProgress::new();
    let pb = m.add(pbar::new(courses.len() as u64)).with_prefix("All");
    let futs = courses.into_iter().map(async |c| -> anyhow::Result<_> {
        let c = c.get().await.context("fetch course")?;
        let announcements = get_announcements(
            &c,
            m.add(pbar::new(0).with_prefix(c.meta().name().to_owned())),
        )
        .await
        .with_context(|| format!("fetch announcement handles of {}", c.meta().title()))?;

        pb.inc_length(announcements.len() as u64);
        let futs = announcements
            .into_iter()
            .map(async |d| -> anyhow::Result<_> {
                pb.inc(1);
                Ok(d)
            });
        let announcements = try_join_all(futs).await?;

        pb.inc(1);
        Ok((c, announcements))
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
    let all_announcements: Vec<_> = courses
        .iter()
        .flat_map(|(c, announcements)| {
            announcements
                .iter()
                .map(move |d| (c.to_owned(), d.id().to_owned(), d.clone()))
        })
        .collect();

    let sorted_announcements = sort_announcements_owned(all_announcements);
    list_brief(sorted_announcements).await
}

async fn list_brief(
    items: Vec<(api::Course, String, api::CourseAnnouncementHandle)>,
) -> anyhow::Result<()> {
    let mut outbuf = Vec::new();
    let title = "课程公告/通知";
    let total = items.len();
    writeln!(outbuf, "{D}>{D:#} {B}{title} ({total}){B:#} {D}<{D:#}\n")?;

    for (idx, (c, id, d)) in items.iter().enumerate() {
        write!(outbuf, "{GR}[{:>2}]{GR:#} ", idx + 1)?;
        write!(outbuf, "{BL}{B}{}{B:#}{BL:#} {D}>{D:#} ", c.meta().name())?;
        write!(outbuf, "{}", d.title())?;
        let att_count = d.attachments().len();
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
    let Some((c, ann_id, d)) = items.into_iter().find(|(_, ann_id, _)| ann_id == id) else {
        anyhow::bail!("announcement with id {} not found", id);
    };

    let mut outbuf = Vec::new();
    writeln!(outbuf, "{D}>{D:#} {B}公告详情{B:#} {D}<{D:#}\n")?;
    write_announcement_title(&mut outbuf, &ann_id, &c, &d).context("io error")?;
    buf_try!(@try fs::stdout().write_all(outbuf).await);
    Ok(())
}

fn sort_announcements_owned(
    mut items: Vec<(api::Course, String, api::CourseAnnouncementHandle)>,
) -> Vec<(api::Course, String, api::CourseAnnouncementHandle)> {
    items.sort_by(|a, b| {
        let time_a = a.2.time();
        let time_b = b.2.time();
        match (time_b, time_a) {
            (Some(t_b), Some(t_a)) => t_b.cmp(t_a), // descending order
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
    });
    items
}

fn sort_announcements_items(mut items: Vec<AnnouncementListItem>) -> Vec<AnnouncementListItem> {
    items.sort_by(|a, b| {
        let time_a = a.2.time();
        let time_b = b.2.time();
        match (time_b, time_a) {
            (Some(t_b), Some(t_a)) => t_b.cmp(t_a), // descending order
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
    });
    items
}

fn write_announcement_title(
    buf: &mut Vec<u8>,
    id: &str,
    c: &api::Course,
    d: &api::CourseAnnouncementHandle,
) -> std::io::Result<()> {
    write!(buf, "{BL}{B}{}{B:#}{BL:#} {D}>{D:#} ", c.meta().name())?;
    write!(buf, "{BL}{B}{}{B:#}{BL:#}", d.title())?;
    let att_count = d.attachments().len();
    if att_count > 0 {
        write!(buf, " ({GR}{att_count} 个附件{GR:#})")?;
    }
    writeln!(buf, " {D}{id}{D:#}")?;

    if !d.descriptions().is_empty() {
        writeln!(buf)?;
        // Show first 3 lines of description as preview
        for (i, p) in d.descriptions().iter().take(3).enumerate() {
            if i == 2 && d.descriptions().len() > 3 {
                writeln!(buf, "{p}...")?;
            } else {
                writeln!(buf, "{p}")?;
            }
        }
        if d.descriptions().len() > 3 {
            writeln!(buf, "... ({GR}{} 行摘要{GR:#})", d.descriptions().len())?;
        }
    }
    if !d.attachments().is_empty() {
        writeln!(buf)?;
        for (name, _) in d.attachments() {
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
        .flat_map(|(c, announcements)| {
            let c = Arc::new(c);
            announcements
                .into_iter()
                .map(move |d| (c.clone(), d.id().to_owned(), d))
        })
        .collect::<Vec<_>>();

    all_announcements = sort_announcements_items(all_announcements);

    Ok(all_announcements)
}
