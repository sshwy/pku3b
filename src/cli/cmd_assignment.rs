use std::sync::Arc;

use anyhow::Context;

use super::*;

async fn get_contents(
    c: &api::Course,
    pb: indicatif::ProgressBar,
) -> anyhow::Result<Vec<api::CourseContent>> {
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
        &format!("get_course_contents_{}", c.meta().id()),
        c.client().cache_ttl(),
        fut,
    )
    .await?;

    Ok(data.into_iter().map(|data| c.build_content(data)).collect())
}

async fn get_assignments(
    c: &api::Course,
    pb: indicatif::ProgressBar,
) -> anyhow::Result<Vec<api::CourseAssignmentHandle>> {
    let r = get_contents(c, pb)
        .await?
        .into_iter()
        .filter_map(|c| c.into_assignment_opt())
        .collect();
    Ok(r)
}

async fn get_courses_and_assignments(
    force: bool,
    cur_term: bool,
) -> anyhow::Result<Vec<(api::Course, Vec<(String, api::CourseAssignment)>)>> {
    let courses = load_courses(force, cur_term).await?;

    // fetch each course concurrently
    let m = indicatif::MultiProgress::new();
    let pb = m.add(pbar::new(courses.len() as u64)).with_prefix("All");
    let futs = courses.into_iter().map(async |c| -> anyhow::Result<_> {
        let c = c.get().await.context("fetch course")?;
        let assignments = get_assignments(
            &c,
            m.add(pbar::new(0).with_prefix(c.meta().name().to_owned())),
        )
        .await
        .with_context(|| format!("fetch assignment handles of {}", c.meta().title()))?;

        pb.inc_length(assignments.len() as u64);
        let futs = assignments.into_iter().map(async |a| -> anyhow::Result<_> {
            let id = a.id();
            let r = a.get().await.context("fetch assignment")?;
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

pub async fn list(force: bool, all: bool, cur_term: bool) -> anyhow::Result<()> {
    let courses = get_courses_and_assignments(force, cur_term).await?;

    let mut all_assignments = courses
        .iter()
        .flat_map(|(c, assignments)| {
            assignments
                .iter()
                .map(move |(id, a)| (c.to_owned(), id.to_owned(), a))
        })
        // retain only unfinished assignments if not in full mode
        .filter(|(_, _, a)| all || a.last_attempt().is_none())
        .collect::<Vec<_>>();

    // sort by deadline
    log::debug!("sorting assignments...");
    all_assignments.sort_by_cached_key(|(_, _, a)| a.deadline());

    // prepare output statements
    let mut outbuf = Vec::new();
    let title = if all {
        "所有作业 (包括已完成)"
    } else {
        "未完成作业"
    };
    let total = all_assignments.len();
    writeln!(outbuf, "{D}>{D:#} {B}{title} ({total}){B:#} {D}<{D:#}\n")?;

    for (c, id, a) in all_assignments {
        write_course_assignment(&mut outbuf, &id, &c, a).context("io error")?;
    }

    // write to stdout
    buf_try!(@try fs::stdout().write_all(outbuf).await);

    Ok(())
}

type AssignmentListItem = (Arc<api::Course>, String, api::CourseAssignment);

async fn fetch_assignments(
    force: bool,
    all: bool,
    cur_term: bool,
) -> anyhow::Result<Vec<AssignmentListItem>> {
    let courses = get_courses_and_assignments(force, cur_term).await?;

    let mut all_assignments = courses
        .into_iter()
        .flat_map(|(c, assignments)| {
            let c = Arc::new(c);
            assignments
                .into_iter()
                .map(move |(id, a)| (c.clone(), id, a))
        })
        // retain only unfinished assignments if not in full mode
        .filter(|(_, _, a)| all || a.last_attempt().is_none())
        .collect::<Vec<_>>();

    // sort by deadline
    log::debug!("sorting assignments...");
    all_assignments.sort_by_cached_key(|(_, _, a)| a.deadline());

    Ok(all_assignments)
}

async fn select_assignment(
    mut items: Vec<AssignmentListItem>,
) -> anyhow::Result<AssignmentListItem> {
    if items.is_empty() {
        anyhow::bail!("assignments not found");
    }

    let mut options = Vec::new();

    for (idx, (c, id, a)) in items.iter().enumerate() {
        let mut outbuf = Vec::new();
        write!(outbuf, "[{}] ", idx + 1)?;
        write_assignment_title_ln(&mut outbuf, id, c, a).context("io error")?;
        options.push(String::from_utf8(outbuf).unwrap());
    }

    let s = inquire::Select::new("请选择要下载的作业", options).raw_prompt()?;
    let idx = s.index;
    let r = items.swap_remove(idx);

    Ok(r)
}

pub async fn download(
    id: Option<&str>,
    dir: &std::path::Path,
    force: bool,
    all: bool,
    cur_term: bool,
) -> anyhow::Result<()> {
    let items = fetch_assignments(force, all, cur_term).await?;
    let a = match id {
        Some(id) => match items.into_iter().find(|x| x.1 == id) {
            Some(r) => r,
            None => anyhow::bail!("assignment with id {} not found", id),
        },
        None => select_assignment(items).await?,
    };

    let sp = pbar::new_spinner();
    download_data(sp, dir, &a.2).await?;

    Ok(())
}

async fn download_data(
    sp: pbar::AsyncSpinner,
    dir: &std::path::Path,
    a: &api::CourseAssignment,
) -> anyhow::Result<()> {
    if !dir.exists() {
        compio::fs::create_dir_all(dir).await?;
    }

    let atts = a.attachments();
    let tot = atts.len();
    for (id, (name, uri)) in atts.iter().enumerate() {
        sp.set_message(format!(
            "[{}/{tot}] downloading attachment '{name}'...",
            id + 1
        ));
        a.download_attachment(uri, &dir.join(name))
            .await
            .with_context(|| format!("download attachment '{name}'"))?;
    }

    drop(sp);
    println!("Done.");
    Ok(())
}

pub async fn submit(id: Option<&str>, path: Option<&std::path::Path>) -> anyhow::Result<()> {
    let items = fetch_assignments(false, false, true).await?;

    let (c, _, a) = match id {
        Some(id) => match items.into_iter().find(|x| x.1 == id) {
            Some(r) => r,
            None => anyhow::bail!("assignment with id {} not found", id),
        },
        None => select_assignment(items).await?,
    };

    let path = match path {
        Some(path) => path.to_owned(),
        None => {
            // list the current dir and use inquire::Select to choose a file

            let mut options = Vec::new();
            // fill options with files in the current dir
            let entries = std::fs::read_dir(".")?;
            for entry in entries {
                let Ok(entry) = entry else {
                    continue;
                };
                let path = entry.path();
                if path.is_file() {
                    options.push(path.to_str().unwrap().to_owned());
                }
            }

            if options.is_empty() {
                anyhow::bail!("no files found in current directory");
            }
            let s = inquire::Select::new("请选择要提交的文件", options).prompt()?;

            s.into()
        }
    };

    if !path.exists() {
        anyhow::bail!("file not found: {:?}", path);
    }

    let sp = pbar::new_spinner();
    sp.set_message("submit file...");
    a.submit_file(path.as_path())
        .await
        .with_context(|| format!("submit {:?} to {:?}", path.display(), a.title()))?;

    drop(sp);

    println!(
        "成功将 {GR}{H2}{}{H2:#}{GR:#} 提交至 {MG}{H1}{} {}{H1:#}{MG:#} 课程作业",
        path.display(),
        c.meta().name(),
        a.title()
    );

    println!("{EM:}tips: 执行 {H2}pku3b a -f ls -a{H2:#} 可强制刷新缓存并查看作业完成状态{EM:#}");
    Ok(())
}

fn write_assignment_title_ln(
    buf: &mut Vec<u8>,
    id: &str,
    c: &api::Course,
    a: &api::CourseAssignment,
) -> std::io::Result<()> {
    write!(buf, "{BL}{B}{}{B:#}{BL:#} {D}>{D:#} ", c.meta().name())?;
    write!(buf, "{BL}{B}{}{B:#}{BL:#}", a.title())?;
    if let Some(att) = a.last_attempt() {
        write!(buf, " ({GR}已完成: {att}{GR:#})")?;
    } else if let Some(t) = a.deadline() {
        let delta = t - chrono::Local::now();
        write!(buf, " ({})", fmt_time_delta(delta))?;
    } else if let Some(raw) = a.deadline_raw() {
        write!(buf, " ({raw})")?;
    } else {
        write!(buf, " (无截止时间)")?;
    }
    writeln!(buf, " {D}{id}{D:#}")?;
    Ok(())
}

fn write_course_assignment(
    buf: &mut Vec<u8>,
    id: &str,
    c: &api::Course,
    a: &api::CourseAssignment,
) -> std::io::Result<()> {
    write_assignment_title_ln(buf, id, c, a)?;

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

pub fn fmt_time_delta(delta: chrono::TimeDelta) -> String {
    use utils::style::*;
    if delta < chrono::TimeDelta::zero() {
        return format!("{RD}due{RD:#}");
    }

    let s = if delta > chrono::TimeDelta::days(1) {
        Style::new().fg_color(Some(AnsiColor::Yellow.into()))
    } else {
        Style::new().fg_color(Some(AnsiColor::Red.into()))
    };

    let mut delta = delta.to_std().unwrap();
    let mut res = String::new();
    res.push_str("in ");
    if delta.as_secs() >= 86400 {
        res.push_str(&format!("{}d ", delta.as_secs() / 86400));
        delta = std::time::Duration::from_secs(delta.as_secs() % 86400);
    }
    if delta.as_secs() >= 3600 {
        res.push_str(&format!("{}h ", delta.as_secs() / 3600));
        delta = std::time::Duration::from_secs(delta.as_secs() % 3600);
    }
    if delta.as_secs() >= 60 {
        res.push_str(&format!("{}m ", delta.as_secs() / 60));
        delta = std::time::Duration::from_secs(delta.as_secs() % 60);
    }
    res.push_str(&format!("{}s", delta.as_secs()));
    format!("{s}{res}{s:#}")
}
