use anyhow::Context;

use super::*;
pub async fn list(force: bool) -> anyhow::Result<()> {
    let courses = load_courses(force).await?;

    let pb = pbar::new(courses.len() as u64);
    let futs = courses.into_iter().map(async |c| -> anyhow::Result<_> {
        let c = c.get().await.context("fetch course")?;
        let vs = c.get_video_list().await.context("fetch video list")?;
        pb.inc(1);
        Ok((c, vs))
    });
    let courses = try_join_all(futs).await?;
    pb.finish_and_clear();

    let mut outbuf = Vec::new();
    let title = "课程回放";

    writeln!(outbuf, "{D}>{D:#} {B}{}{B:#} {D}<{D:#}\n", title)?;

    for (c, vs) in courses {
        if vs.is_empty() {
            continue;
        }

        writeln!(outbuf, "{BL}{H1}[{}]{H1:#}{BL:#}\n", c.meta().title())?;

        for v in vs {
            writeln!(
                outbuf,
                "{D}•{D:#} {} ({}) {D}{}{D:#}",
                v.meta().title(),
                v.meta().time(),
                v.id()
            )?;
        }

        writeln!(outbuf)?;
    }

    buf_try!(@try fs::stdout().write_all(outbuf).await);
    Ok(())
}

pub async fn download(force: bool, id: String) -> anyhow::Result<()> {
    let (_, courses, sp) = load_client_courses(force).await?;

    sp.set_message("finding video...");
    let mut target_video = None;
    for c in courses {
        let c = c.get().await.context("fetch course")?;

        let vs = c.get_video_list().await?;
        for v in vs {
            if v.id() == id {
                target_video = Some(v);
                break;
            }
        }

        if target_video.is_some() {
            break;
        }
    }
    let Some(v) = target_video else {
        anyhow::bail!("video with id {} not found", id);
    };

    sp.set_message("fetch video metadata...");
    let v = v.get().await?;

    drop(sp);

    println!("下载课程回放：{} ({})", v.course_name(), v.meta().title());

    // prepare download dir
    let dir = utils::projectdir()
        .cache_dir()
        .join("video_download")
        .join(&id);
    fs::create_dir_all(&dir)
        .await
        .context("create dir failed")?;

    let paths = download_segments(&v, &dir)
        .await
        .context("download ts segments")?;

    let m3u8 = dir.join("playlist").with_extension("m3u8");
    buf_try!(@try fs::write(&m3u8, v.m3u8_raw()).await);

    // merge all segments into one file
    let merged = dir.join("merged").with_extension("ts");
    merge_segments(&merged, &paths).await?;
    let dest = format!("{}_{}.mp4", v.course_name(), v.meta().title());
    log::info!("Merged segments to {}", merged.display());
    log::info!(
        r#"You may execute `ffmpeg -i "{}" -c copy "{}"` to convert it to mp4"#,
        merged.display(),
        dest,
    );

    // convert the merged ts file to mp4. overwrite existing file
    let sp = pbar::new_spinner();
    sp.set_message("Converting to mp4 file...");
    let c = compio::process::Command::new("ffmpeg")
        .args(["-y", "-hide_banner", "-loglevel", "quiet"])
        .args(["-i", merged.to_string_lossy().as_ref()])
        .args(["-c", "copy"])
        .arg(&dest)
        .output()
        .await
        .context("execute ffmpeg")?;
    drop(sp);

    if c.status.success() {
        println!("下载完成, 文件保存为: {GR}{H2}{}{H2:#}{GR:#}", dest);
    } else {
        anyhow::bail!("ffmpeg failed with exit code {:?}", c.status.code());
    }

    Ok(())
}

async fn download_segments(
    v: &api::CourseVideo,
    dir: impl AsRef<std::path::Path>,
) -> anyhow::Result<Vec<std::path::PathBuf>> {
    let dir = dir.as_ref();
    if !dir.exists() {
        anyhow::bail!("dir {} not exists", dir.display());
    }

    let tot = v.len_segments();
    let pb = pbar::new(tot as u64).with_prefix("download");
    pb.tick();

    let mut key = None;
    let mut paths = Vec::new();
    // faster than try_join_all
    for i in 0..tot {
        key = v.refresh_key(i, key);
        let path = dir.join(&v.segment(i).uri).with_extension("ts");

        if !path.exists() {
            log::debug!("key: {:?}", key);
            let seg = v
                .get_segment_data(i, key)
                .await
                .with_context(|| format!("get segment #{i} with key {key:?}"))?;

            // fs::write is not atomic, so we write to a tmp file first
            let tmpath = path.with_extension("tmp");
            buf_try!(@try fs::write(&tmpath, seg).await);
            fs::rename(tmpath, &path).await.context("rename tmp file")?;
        }

        pb.inc(1);
        paths.push(path);
    }
    pb.finish_and_clear();

    Ok(paths)
}

async fn merge_segments(
    dest: impl AsRef<std::path::Path>,
    paths: &[std::path::PathBuf],
) -> anyhow::Result<()> {
    let f = fs::File::create(&dest)
        .await
        .context("create merged file failed")?;
    let mut f = std::io::Cursor::new(f);

    let pb = pbar::new(paths.len() as u64).with_prefix("merge segments");
    pb.tick();
    for p in paths {
        let data = fs::read(p).await.context("read segments failed")?;
        buf_try!(@try f.write(data).await);
        pb.inc(1);
    }
    pb.finish_and_clear();

    Ok(())
}
