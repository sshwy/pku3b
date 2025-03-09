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

            write_course_assignment(&mut outbuf, &id, &a)?;
        }
    }

    // write to stdout
    buf_try!(@try fs::stdout().write_all(outbuf).await);

    Ok(())
}

pub async fn download(id: &str, dir: &std::path::Path) -> anyhow::Result<()> {
    let (_, courses, sp) = load_client_courses(false).await?;

    let mut target_handle = None;

    sp.set_message("finding assignment...");
    for c in courses {
        let c = c.get().await.context("fetch course")?;
        let assignments = c
            .get_assignments()
            .await
            .with_context(|| format!("fetch assignment handles of {}", c.meta().title()))?;

        for a in assignments {
            if a.id() == id {
                target_handle = Some(a);
                break;
            }
        }

        if target_handle.is_some() {
            break;
        }
    }

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

fn write_course_assignment(
    buf: &mut Vec<u8>,
    id: &str,
    a: &api::CourseAssignment,
) -> anyhow::Result<()> {
    write!(buf, "{MG}{H2}{}{H2:#}{MG:#}", a.title())?;
    if let Some(att) = a.last_attempt() {
        write!(buf, " ({GR}已完成{GR:#}) {D}{att}{D:#}")?;
    } else {
        let t = a
            .deadline()
            .with_context(|| format!("fail to parse deadline: {}", a.deadline_raw()))?;
        let delta = t - chrono::Local::now();
        write!(buf, " ({})", fmt_time_delta(delta))?;
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
