use super::*;
pub async fn run(force: bool, all: bool) -> anyhow::Result<()> {
    let client = api::Client::new(
        if force { None } else { Some(ONE_HOUR) },
        if force { None } else { Some(ONE_DAY) },
    );

    let pb = pbar::new_spinner();

    pb.set_message("reading config...");
    let cfg_path = utils::default_config_path();
    let cfg = config::read_cfg(cfg_path)
        .await
        .context("read config file")?;

    pb.set_message("logging in to blackboard...");
    let blackboard = client
        .blackboard(&cfg.username, &cfg.password)
        .await
        .context("login to blackboard")?;

    pb.set_message("fetching courses...");
    let courses = blackboard
        .get_courses()
        .await
        .context("fetch course handles")?;

    pb.finish_and_clear().await;

    // fetch each course concurrently
    let pb = pbar::new(courses.len() as u64);
    let futs = courses.into_iter().map(async |c| -> anyhow::Result<_> {
        let c = c.get().await.context("fetch course")?;
        let assignments = c
            .get_assignments()
            .await
            .with_context(|| format!("fetch assignment handles of {}", c.meta().title()))?;

        pb.inc_length(assignments.len() as u64);
        let futs = assignments.into_iter().map(async |a| {
            let r = a.get().await.context("fetch assignment");
            pb.inc(1);
            r
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
        for a in assignments {
            // skip finished assignments if not in full mode
            if a.last_attempt().is_some() && !all {
                continue;
            }

            write_course_assignments(&mut outbuf, &a)?;
        }
    }

    // write to stdout
    buf_try!(@try fs::stdout().write_all(outbuf).await);

    Ok(())
}

fn write_course_assignments(buf: &mut Vec<u8>, a: &api::CourseAssignment) -> anyhow::Result<()> {
    if let Some(att) = a.last_attempt() {
        writeln!(
            buf,
            "{MG}{H2}{}{H2:#}{MG:#} ({GR}已完成{GR:#}) {D}{att}{D:#}\n",
            a.title()
        )?;
    } else {
        let t = a
            .deadline()
            .with_context(|| format!("fail to parse deadline: {}", a.deadline_raw()))?;
        let delta = t - chrono::Local::now();
        writeln!(
            buf,
            "{MG}{H2}{}{H2:#}{MG:#} ({})\n",
            a.title(),
            fmt_time_delta(delta),
        )?;
    }
    if !a.attachments().is_empty() {
        writeln!(buf, "{H3}附件{H3:#}")?;
        for (name, uri) in a.attachments() {
            writeln!(buf, "{D}•{D:#} {name}: {D}{uri}{D:#}")?;
        }
        writeln!(buf,)?;
    }
    if !a.descriptions().is_empty() {
        writeln!(buf, "{H3}描述{H3:#}")?;
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
