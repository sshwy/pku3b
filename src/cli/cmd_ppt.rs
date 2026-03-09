use std::sync::Arc;

use anyhow::Context;

use super::cmd_assignment::get_contents;
use super::*;

async fn get_ppts(
    c: &api::Course,
    pb: indicatif::ProgressBar,
) -> anyhow::Result<Vec<api::CoursePPTHandle>> {
    let r = get_contents(c, pb)
        .await?
        .into_iter()
        .filter_map(|c| c.into_ppt_opt())
        .collect();
    Ok(r)
}

async fn get_courses_and_ppts(
    force: bool,
    cur_term: bool,
) -> anyhow::Result<Vec<(api::Course, Vec<(String, api::CoursePPT)>)>> {
    let courses = load_courses(force, cur_term).await?;

    // fetch each course concurrently
    let m = indicatif::MultiProgress::new();
    let pb = m.add(pbar::new(courses.len() as u64)).with_prefix("All");
    let futs = courses.into_iter().map(async |c| -> anyhow::Result<_> {
        let c = c.get().await.context("fetch course")?;
        let assignments = get_ppts(
            &c,
            m.add(pbar::new(0).with_prefix(c.meta().name().to_owned())),
        )
        .await
        .with_context(|| format!("fetch assignment handles of {}", c.meta().title()))?;

        pb.inc_length(assignments.len() as u64);
        let futs = assignments.into_iter().map(async |a| -> anyhow::Result<_> {
            let id = a.id();
            let r = a.get().context("fetch assignment")?;
            pb.inc(1);
            Ok((id, r))
        });
        let assignments = try_join_all(futs).await?;

        pb.inc(1);
        Ok((c, assignments))
    });
    let courses = try_join_all(futs).await?;
    pb.finish_and_clear();
    m.clear().unwrap();
    drop(pb);
    drop(m);

    Ok(courses)
}

pub async fn list(force: bool, cur_term: bool) -> anyhow::Result<()> {
    let courses = get_courses_and_ppts(force, cur_term).await?;

    let all_ppts = courses
        .iter()
        .flat_map(|(c, assignments)| {
            assignments
                .iter()
                .map(move |(id, a)| (c.to_owned(), id.to_owned(), a))
        })
        .collect::<Vec<_>>();

    // prepare output statements
    let mut outbuf = Vec::new();
    let title = "PPT列表";
    let total = all_ppts.len();
    writeln!(outbuf, "{D}>{D:#} {B}{title} ({total}){B:#} {D}<{D:#}\n")?;

    for (c, id, a) in all_ppts {
        write_course_ppt(&mut outbuf, &id, &c, a).context("io error")?;
    }

    // write to stdout
    buf_try!(@try fs::stdout().write_all(outbuf).await);

    Ok(())
}

type PPTListItem = (Arc<api::Course>, String, api::CoursePPT);

fn has_known_suffix(path: &std::path::Path) -> bool {
    let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
        return false;
    };
    matches!(
        &ext.to_ascii_lowercase()[..],
        "ppt" | "pptx" | "doc" | "docx" | "pdf"
    )
}

fn detect_attachment_ext(path: &std::path::Path) -> anyhow::Result<Option<&'static str>> {
    let data = std::fs::read(path)?;

    if data.starts_with(b"%PDF-") {
        return Ok(Some("pdf"));
    }

    const OLE_MAGIC: &[u8; 8] = b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1";
    if data.starts_with(OLE_MAGIC) {
        if data
            .windows(b"PowerPoint Document".len())
            .any(|w| w == b"PowerPoint Document")
            || data
                .windows(b"Current User".len())
                .any(|w| w == b"Current User")
        {
            return Ok(Some("ppt"));
        }
        if data
            .windows(b"WordDocument".len())
            .any(|w| w == b"WordDocument")
        {
            return Ok(Some("doc"));
        }
        return Ok(None);
    }

    if data.starts_with(b"PK\x03\x04")
        || data.starts_with(b"PK\x05\x06")
        || data.starts_with(b"PK\x07\x08")
    {
        if data.windows(b"ppt/".len()).any(|w| w == b"ppt/") {
            return Ok(Some("pptx"));
        }
        if data.windows(b"word/".len()).any(|w| w == b"word/") {
            return Ok(Some("docx"));
        }
        return Ok(None);
    }

    Ok(None)
}

async fn fetch_ppts(force: bool, cur_term: bool) -> anyhow::Result<Vec<PPTListItem>> {
    let courses = get_courses_and_ppts(force, cur_term).await?;

    let all_assignments = courses
        .into_iter()
        .flat_map(|(c, assignments)| {
            let c = Arc::new(c);
            assignments
                .into_iter()
                .map(move |(id, a)| (c.clone(), id, a))
        })
        .collect::<Vec<_>>();

    Ok(all_assignments)
}

async fn select_ppt(mut items: Vec<PPTListItem>) -> anyhow::Result<PPTListItem> {
    if items.is_empty() {
        anyhow::bail!("assignments not found");
    }

    let mut options = Vec::new();

    for (idx, (c, id, a)) in items.iter().enumerate() {
        let mut outbuf = Vec::new();
        write!(outbuf, "[{}] ", idx + 1)?;
        write_ppt_title_ln(&mut outbuf, id, c, a).context("io error")?;
        options.push(String::from_utf8(outbuf).unwrap());
    }

    let s = inquire::Select::new("请选择要下载的作业", options).raw_prompt()?;
    let idx = s.index;
    let r = items.swap_remove(idx);

    Ok(r)
}

fn select_course_name(items: &[PPTListItem]) -> anyhow::Result<String> {
    if items.is_empty() {
        anyhow::bail!("ppts not found");
    }

    let mut options = Vec::new();
    for (course, _, _) in items {
        let name = course.meta().name().to_owned();
        if !options.contains(&name) {
            options.push(name);
        }
    }

    if options.is_empty() {
        anyhow::bail!("courses not found");
    }

    let chosen = inquire::Select::new("请选择课程", options).prompt()?;
    Ok(chosen)
}

fn sanitize_path_component(raw: &str) -> String {
    let cleaned = raw
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect::<String>()
        .trim()
        .to_owned();

    if cleaned.is_empty() {
        "untitled".to_owned()
    } else {
        cleaned
    }
}

fn next_available_path(path: &std::path::Path) -> std::path::PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }

    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = path.extension().and_then(|s| s.to_str());
    let parent = path.parent().unwrap_or_else(|| std::path::Path::new(""));

    let mut i = 1usize;
    loop {
        let candidate_name = match ext {
            Some(ext) if !ext.is_empty() => format!("{stem}({i}).{ext}"),
            _ => format!("{stem}({i})"),
        };
        let candidate = parent.join(candidate_name);
        if !candidate.exists() {
            return candidate;
        }
        i += 1;
    }
}

fn select_ppts_by_course(
    items: Vec<PPTListItem>,
    course_name: &str,
) -> anyhow::Result<Vec<PPTListItem>> {
    let exact = items
        .iter()
        .filter(|(c, _, _)| c.meta().name() == course_name)
        .count();

    if exact > 0 {
        let selected = items
            .into_iter()
            .filter(|(c, _, _)| c.meta().name() == course_name)
            .collect::<Vec<_>>();
        return Ok(selected);
    }

    let selected = items
        .into_iter()
        .filter(|(c, _, _)| c.meta().name().contains(course_name))
        .collect::<Vec<_>>();

    if selected.is_empty() {
        anyhow::bail!("no course matched: {course_name}");
    }

    Ok(selected)
}

pub async fn download(
    id: Option<&str>,
    dir: &std::path::Path,
    force: bool,
    cur_term: bool,
    overwrite: bool,
    suffix_on_conflict: bool,
) -> anyhow::Result<()> {
    let items = fetch_ppts(force, cur_term).await?;

    let a = match id {
        Some(id) => match items.into_iter().find(|x| x.1 == id) {
            Some(r) => r,
            None => anyhow::bail!("assignment with id {} not found", id),
        },
        None => select_ppt(items).await?,
    };

    let sp = pbar::new_spinner();
    download_data(sp, dir, &a.2, overwrite, suffix_on_conflict).await?;

    Ok(())
}

pub async fn download_course(
    dir: &std::path::Path,
    force: bool,
    cur_term: bool,
    overwrite: bool,
    suffix_on_conflict: bool,
) -> anyhow::Result<()> {
    let items = fetch_ppts(force, cur_term).await?;
    let course = select_course_name(&items)?;
    let selected = select_ppts_by_course(items, &course)?;

    for (_, _ppt_id, ppt) in selected.into_iter() {
        let sp = pbar::new_spinner();
        sp.set_message(format!("downloading '{}'", ppt.title()));
        download_data(sp, dir, &ppt, overwrite, suffix_on_conflict).await?;
    }

    Ok(())
}

async fn download_data(
    sp: pbar::AsyncSpinner,
    dir: &std::path::Path,
    a: &api::CoursePPT,
    overwrite: bool,
    suffix_on_conflict: bool,
) -> anyhow::Result<()> {
    if !dir.exists() {
        compio::fs::create_dir_all(dir).await?;
    }

    let atts = a.attachments();
    let tot = atts.len();
    for (id, (name, uri)) in atts.iter().enumerate() {
        let mut target = dir.join(sanitize_path_component(name));
        if !overwrite && target.exists() && suffix_on_conflict {
            target = next_available_path(&target);
        } else if !overwrite && target.exists() {
            sp.set_message(format!(
                "[{}/{tot}] skipping existing attachment '{name}'...",
                id + 1
            ));
            continue;
        }

        sp.set_message(format!(
            "[{}/{tot}] downloading attachment '{name}'...",
            id + 1
        ));
        a.download_attachment(uri, &target)
            .await
            .with_context(|| format!("download attachment '{name}'"))?;

        if !has_known_suffix(&target)
            && let Some(ext) = detect_attachment_ext(&target)
                .with_context(|| format!("detect attachment type for '{name}'"))?
        {
            let mut renamed =
                target.with_file_name(format!("{}.{ext}", sanitize_path_component(name)));
            if !overwrite && renamed.exists() && suffix_on_conflict {
                renamed = next_available_path(&renamed);
            }
            if overwrite || !renamed.exists() {
                std::fs::rename(&target, &renamed)
                    .with_context(|| format!("rename attachment '{name}' to add .{ext}"))?;
            }
        }
    }

    drop(sp);
    println!("Done.");
    Ok(())
}

fn write_ppt_title_ln(
    buf: &mut Vec<u8>,
    id: &str,
    c: &api::Course,
    a: &api::CoursePPT,
) -> std::io::Result<()> {
    write!(buf, "{BL}{B}{}{B:#}{BL:#} {D}>{D:#} ", c.meta().name())?;
    write!(buf, "{BL}{B}{}{B:#}{BL:#}", a.title())?;
    writeln!(buf, " {D}{id}{D:#}")?;
    Ok(())
}

fn write_course_ppt(
    buf: &mut Vec<u8>,
    id: &str,
    c: &api::Course,
    a: &api::CoursePPT,
) -> std::io::Result<()> {
    write_ppt_title_ln(buf, id, c, a)?;

    if !a.descriptions().is_empty() {
        writeln!(buf)?;
        for p in a.descriptions() {
            writeln!(buf, "{p}")?;
        }
    }
    if !a.attachments().is_empty() {
        writeln!(buf)?;
        for (name, _) in a.attachments() {
            writeln!(buf, "{D}[附件]{D:#} {UL}{name}{UL:#}")?;
        }
    }
    writeln!(buf)?;

    Ok(())
}
