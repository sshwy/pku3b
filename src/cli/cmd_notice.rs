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

async fn get_contents(
    c: &api::Course,
    pb: indicatif::ProgressBar,
) -> anyhow::Result<Vec<api::CourseContent>> {
    let fut = async {
        let mut s = c.content_stream();

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
        &format!("get_course_announcements_{}", c.meta().id()),
        c.client().cache_ttl(),
        fut,
    )
    .await?;

    Ok(data.into_iter().map(|data| c.build_content(data)).collect())
}

async fn get_announcements(
    c: &api::Course,
    pb: indicatif::ProgressBar,
) -> anyhow::Result<Vec<api::CourseAnnouncementHandle>> {
    let r = get_contents(c, pb)
        .await?
        .into_iter()
        .filter_map(|c| c.into_announcement_opt())
        .collect();
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
        let futs = announcements.into_iter().map(async |d| -> anyhow::Result<_> {
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

    let all_announcements = courses
        .iter()
        .flat_map(|(c, announcements)| {
            announcements
                .iter()
                .map(move |d| (c.to_owned(), d.id().to_owned(), d.clone()))
        })
        .collect::<Vec<_>>();

    let mut sorted_announcements = all_announcements;

    // sort by title
    log::debug!("sorting announcements...");
    sorted_announcements.sort_by_cached_key(|(_, _, d)| d.title().to_string());

    // prepare output statements
    let mut outbuf = Vec::new();
    let title = "课程公告/通知";
    let total = sorted_announcements.len();
    writeln!(outbuf, "{D}>{D:#} {B}{title} ({total}){B:#} {D}<{D:#}\n")?;

    for (c, id, d) in sorted_announcements {
        write_announcement_title(&mut outbuf, &id, &c, &d).context("io error")?;
    }

    // write to stdout
    buf_try!(@try fs::stdout().write_all(outbuf).await);

    Ok(())
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

    // sort by title
    log::debug!("sorting announcements...");
    all_announcements.sort_by_cached_key(|(_, _, d)| d.title().to_string());

    Ok(all_announcements)
}

pub async fn select_announcement(
    mut items: Vec<AnnouncementListItem>,
) -> anyhow::Result<AnnouncementListItem> {
    if items.is_empty() {
        anyhow::bail!("announcements not found");
    }

    let mut options = Vec::new();

    for (idx, (c, id, d)) in items.iter().enumerate() {
        let mut outbuf = Vec::new();
        write!(outbuf, "[{}] ", idx + 1)?;
        write_announcement_title(&mut outbuf, id, c, d).context("io error")?;
        options.push(String::from_utf8(outbuf).unwrap());
    }

    let s = inquire::Select::new("请选择要查看的公告", options).raw_prompt()?;
    let idx = s.index;
    let r = items.swap_remove(idx);

    Ok(r)
}
