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
        .with_context(|| format!("fetch announcements from course page for {}", c.meta().title()))?;
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

pub async fn list(force: bool, cur_term: bool, brief: bool, interactive: bool) -> anyhow::Result<()> {
    let courses = get_courses_and_announcements(force, cur_term).await?;

    let all_announcements: Vec<_> = courses
        .iter()
        .flat_map(|(c, announcements)| {
            announcements
                .iter()
                .map(move |d| (c.to_owned(), d.id().to_owned(), d.clone()))
        })
        .collect();

    let mut sorted_announcements = all_announcements;

    // sort by title
    log::debug!("sorting announcements...");
    sorted_announcements.sort_by_cached_key(|(_, _, d)| d.title().to_string());

    if interactive {
        // 交互模式：单条浏览
        list_interactive(sorted_announcements).await
    } else if brief {
        // 简洁模式：只显示标题
        list_brief(sorted_announcements).await
    } else {
        // 默认模式：完整输出
        list_full(sorted_announcements).await
    }
}

async fn list_brief(items: Vec<(api::Course, String, api::CourseAnnouncementHandle)>) -> anyhow::Result<()> {
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

async fn list_full(items: Vec<(api::Course, String, api::CourseAnnouncementHandle)>) -> anyhow::Result<()> {
    let mut outbuf = Vec::new();
    let title = "课程公告/通知";
    let total = items.len();
    writeln!(outbuf, "{D}>{D:#} {B}{title} ({total}){B:#} {D}<{D:#}\n")?;

    for (c, id, d) in items {
        write_announcement_title(&mut outbuf, &id, &c, &d).context("io error")?;
    }

    buf_try!(@try fs::stdout().write_all(outbuf).await);
    Ok(())
}

async fn list_interactive(items: Vec<(api::Course, String, api::CourseAnnouncementHandle)>) -> anyhow::Result<()> {
    use inquire::{InquireError, Select};
    
    if items.is_empty() {
        println!("暂无公告");
        return Ok(());
    }

    let total = items.len();
    let mut current_idx = 0;

    loop {
        let (c, id, d) = &items[current_idx];
        
        // 清屏（Unix-like 系统）
        print!("\x1B[2J\x1B[H");
        
        // 显示当前公告
        let mut outbuf = Vec::new();
        writeln!(outbuf, "{D}>{D:#} {B}公告 {}/{}{B:#} {D}<{D:#}\n", current_idx + 1, total)?;
        write_announcement_title(&mut outbuf, id, c, d).context("io error")?;
        buf_try!(@try fs::stdout().write_all(outbuf).await);

        // 构建选项
        let mut options = vec!["[退出]"];
        if current_idx > 0 {
            options.push("[← 上一条]");
        }
        if current_idx < total - 1 {
            options.push("[下一条 →]");
        }
        
        let selection = Select::new("请选择操作", options).prompt();
        
        match selection {
            Ok("[退出]") | Err(InquireError::OperationCanceled) | Err(InquireError::OperationInterrupted) => break,
            Ok("[← 上一条]") if current_idx > 0 => current_idx -= 1,
            Ok("[下一条 →]") if current_idx < total - 1 => current_idx += 1,
            _ => {}
        }
    }

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
