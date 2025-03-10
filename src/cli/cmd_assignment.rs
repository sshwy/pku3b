use anyhow::Context;

use super::*;
pub async fn list(force: bool, all: bool) -> anyhow::Result<()> {
    let courses = load_courses(force).await?;

    // fetch each course concurrently
    let pb = pbar::new(courses.len() as u64);
    let futs = courses.into_iter().map(async |c| -> anyhow::Result<_> {
        let c = c.get().await.context("fetch course")?;
        let assignments = c
            .get_assignments()
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

    // prepare output statements
    let mut outbuf = Vec::new();
    let title = if all {
        "所有作业 (包括已完成)"
    } else {
        "未完成作业"
    };
    writeln!(outbuf, "{D}>{D:#} {B}{}{B:#} {D}<{D:#}\n", title)?;

    for (c, assignments) in courses {
        if assignments.is_empty() {
            continue;
        }

        writeln!(outbuf, "{BL}{H1}[{}]{H1:#}{BL:#}\n", c.meta().title())?;
        for (id, a) in assignments {
            // skip finished assignments if not in full mode
            if a.last_attempt().is_some() && !all {
                continue;
            }

            write_course_assignment(&mut outbuf, &id, &a).context("io error")?;
        }
    }

    // write to stdout
    buf_try!(@try fs::stdout().write_all(outbuf).await);

    Ok(())
}

pub async fn find_assignment(
    courses: &[api::CourseHandle],
    id: &str,
) -> anyhow::Result<Option<api::CourseAssignmentHandle>> {
    for c in courses {
        let c = c.get().await.context("fetch course")?;
        let assignments = c
            .get_assignments()
            .await
            .with_context(|| format!("fetch assignment handles of {}", c.meta().title()))?;

        for a in assignments {
            if a.id() == id {
                return Ok(Some(a));
            }
        }
    }
    Ok(None)
}

pub async fn download(id: &str, dir: &std::path::Path) -> anyhow::Result<()> {
    let (_, courses, sp) = load_client_courses(false).await?;

    sp.set_message("finding assignment...");
    let target_handle = find_assignment(&courses, id).await?;

    let Some(a) = target_handle else {
        sp.finish_and_clear().await;
        anyhow::bail!("assignment with id {} not found", id);
    };

    sp.set_message("fetch assignment metadata...");
    let a = a.get().await?;

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
            .with_context(|| format!("download attachment '{}'", name))?;
    }

    sp.finish_and_clear().await;
    println!("Done.");
    Ok(())
}

pub async fn submit(id: &str, path: &std::path::Path) -> anyhow::Result<()> {
    if !path.exists() {
        anyhow::bail!("file not found: {:?}", path);
    }
    let (_, courses, sp) = load_client_courses(false).await?;

    let target_handle = cmd_assignment::find_assignment(&courses, id).await?;

    let Some(a) = target_handle else {
        sp.finish_and_clear().await;
        anyhow::bail!("assignment with id {} not found", id);
    };

    sp.set_message("fetch assignment metadata...");
    let a = a.get().await?;

    sp.set_message("submit file...");
    a.submit_file(path).await.context("submit file")?;

    sp.finish_and_clear().await;
    Ok(())
}

fn write_course_assignment(
    buf: &mut Vec<u8>,
    id: &str,
    a: &api::CourseAssignment,
) -> std::io::Result<()> {
    write!(buf, "{MG}{H2}{}{H2:#}{MG:#}", a.title())?;
    if let Some(att) = a.last_attempt() {
        write!(buf, " ({GR}已完成: {att}{GR:#})")?;
    } else {
        if let Some(t) = a.deadline() {
            let delta = t - chrono::Local::now();
            write!(buf, " ({})", fmt_time_delta(delta))?;
        } else if let Some(raw) = a.deadline_raw() {
            write!(buf, " ({})", raw)?;
        } else {
            write!(buf, " (无截止时间)")?;
        }
    }
    writeln!(buf, " {D}{}{D:#}", id)?;

    if !a.attachments().is_empty() {
        writeln!(buf, "\n{H3}附件{H3:#}")?;
        for (name, _) in a.attachments() {
            writeln!(buf, "{D}•{D:#} {name}")?;
        }
    }
    if !a.descriptions().is_empty() {
        writeln!(buf, "\n{H3}描述{H3:#}")?;
        for p in a.descriptions() {
            writeln!(buf, "{p}")?;
        }
    }
    writeln!(buf)?;

    Ok(())
}

pub fn fmt_time_delta(delta: chrono::TimeDelta) -> String {
    if delta < chrono::TimeDelta::zero() {
        let s = Style::new().fg_color(Some(AnsiColor::Red.into()));
        return format!("{s}due{s:#}");
    }

    let s = Style::new().fg_color(Some(AnsiColor::Yellow.into()));
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
    format!("{s}{}{s:#}", res)
}
