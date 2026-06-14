use anyhow::Context;

use super::*;

#[derive(clap::Args)]
pub struct CommandVideo {
    /// 强制刷新
    #[arg(short, long, default_value = "false")]
    force: bool,

    #[command(subcommand)]
    command: VideoCommands,

    /// 手机令牌码。当需要使用 OTP 登录，但未提供此参数时，将会从命令行交互式读取 OTP 码。
    #[arg(long, default_value = "")]
    otp_code: String,
}

#[derive(Subcommand)]
enum VideoCommands {
    /// 获取课程回放列表
    #[command(visible_alias("ls"))]
    List {
        /// 显示所有学期的课程回放
        #[arg(long, default_value = "false")]
        all_term: bool,
    },

    /// 下载课程回放视频 (MP4 格式)，支持断点续传
    #[command(visible_alias("down"))]
    #[cfg(feature = "video-download")]
    Download {
        /// 课程回放 ID (形如 `e780808c9eb81f61`, 可通过 `pku3b video list` 查看)
        id: String,

        /// 在所有学期的课程回放范围中查找
        #[arg(long, default_value = "false")]
        all_term: bool,

        /// 文件下载目录 (支持相对路径)
        #[arg(short = 'o', long)]
        outdir: Option<std::path::PathBuf>,
    },

    /// 顺序下载匹配课程的全部课程回放视频
    #[command(visible_alias("down-course"))]
    #[cfg(feature = "video-download")]
    DownloadCourse {
        /// 课程标题关键字；匹配多门课程时会交互选择
        course: String,

        /// 在所有学期的课程范围中查找
        #[arg(long, default_value = "false")]
        all_term: bool,

        /// 文件下载目录 (支持相对路径)
        #[arg(short = 'o', long)]
        outdir: Option<std::path::PathBuf>,
    },
}

pub async fn run(cmd: CommandVideo, ctx: &CommandCtx<'_>) -> anyhow::Result<()> {
    match cmd.command {
        VideoCommands::List { all_term } => list(ctx, cmd.force, !all_term, cmd.otp_code).await?,
        #[cfg(feature = "video-download")]
        VideoCommands::Download {
            outdir,
            id,
            all_term,
        } => {
            download(
                ctx,
                outdir.as_deref(),
                cmd.force,
                id,
                !all_term,
                cmd.otp_code,
            )
            .await?
        }
        #[cfg(feature = "video-download")]
        VideoCommands::DownloadCourse {
            course,
            outdir,
            all_term,
        } => {
            download_course(
                ctx,
                outdir.as_deref(),
                cmd.force,
                &course,
                !all_term,
                cmd.otp_code,
            )
            .await?
        }
    }
    Ok(())
}

pub async fn list(
    ctx: &CommandCtx<'_>,
    force: bool,
    cur_term: bool,
    otp_code: String,
) -> anyhow::Result<()> {
    let courses = load_courses(ctx, force, cur_term, otp_code).await?;

    let pb = ctx
        .multi
        .add(pbar::new(courses.len() as u64))
        .with_prefix("All");
    let futs = courses.into_iter().map(async |c| -> anyhow::Result<_> {
        let c = c.get().await.context("fetch course")?;
        let vs = c.get_video_list().await.context("fetch video list")?;
        pb.inc(1);
        Ok((c, vs))
    });
    let courses = try_join_all(futs).await?;
    pb.finish_and_clear();
    ctx.multi.remove(&pb);

    let mut outbuf = Vec::new();
    let title = "课程回放";

    writeln!(outbuf, "{D}>{D:#} {B}{title}{B:#} {D}<{D:#}\n")?;

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

#[cfg(feature = "video-download")]
pub async fn download(
    ctx: &CommandCtx<'_>,
    outdir: Option<&std::path::Path>,
    force: bool,
    id: String,
    cur_term: bool,
    otp_code: String,
) -> anyhow::Result<()> {
    let outdir = prepare_video_outdir(outdir).await?;

    let (_, courses, sp) = load_client_courses(ctx, force, cur_term, otp_code).await?;

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

    ctx.remove_spinner(sp);

    download_course_video(ctx, &v, &id, &outdir).await?;

    Ok(())
}

#[cfg(feature = "video-download")]
pub async fn download_course(
    ctx: &CommandCtx<'_>,
    outdir: Option<&std::path::Path>,
    force: bool,
    course_query: &str,
    cur_term: bool,
    otp_code: String,
) -> anyhow::Result<()> {
    let outdir = prepare_video_outdir(outdir).await?;

    let courses = load_courses(ctx, force, cur_term, otp_code).await?;
    let mut matches = courses
        .into_iter()
        .filter(|c| c.id() == course_query || c.long_title().contains(course_query))
        .collect::<Vec<_>>();

    if matches.is_empty() {
        anyhow::bail!("course matching '{course_query}' not found");
    }

    let course = if matches.len() == 1 {
        matches.swap_remove(0)
    } else {
        let options = matches
            .iter()
            .map(|c| format!("{} {}", c.long_title(), c.id()))
            .collect::<Vec<_>>();
        let selected = inquire::Select::new("请选择要下载回放的课程", options).raw_prompt()?;
        matches.swap_remove(selected.index)
    };

    let sp = ctx.spinner();
    sp.set_message("fetching course...");
    let course = course.get().await.context("fetch course")?;

    sp.set_message("fetching video list...");
    let videos = course.get_video_list().await.context("fetch video list")?;
    ctx.remove_spinner(sp);

    if videos.is_empty() {
        anyhow::bail!("course '{}' has no videos", course.meta().title());
    }

    println!(
        "准备下载 {B}{}{B:#} 的 {B}{}{B:#} 个课程回放到 {}",
        course.meta().title(),
        videos.len(),
        outdir.display()
    );

    let total = videos.len();
    for (idx, video) in videos.into_iter().enumerate() {
        let id = video.id();
        println!("\n[{}/{}] {}", idx + 1, total, video.meta().title());
        let video = video
            .get()
            .await
            .with_context(|| format!("fetch video metadata for {}", id))?;
        download_course_video(ctx, &video, &id, &outdir)
            .await
            .with_context(|| format!("download video {}", id))?;
    }

    println!("全部课程回放下载完成。");

    Ok(())
}

#[cfg(feature = "video-download")]
async fn prepare_video_outdir(
    outdir: Option<&std::path::Path>,
) -> anyhow::Result<std::path::PathBuf> {
    let outdir = outdir.unwrap_or(std::path::Path::new("."));
    fs::create_dir_all(outdir)
        .await
        .with_context(|| format!("create output directory {}", outdir.display()))?;
    Ok(outdir.to_path_buf())
}

#[cfg(feature = "video-download")]
async fn download_course_video(
    ctx: &CommandCtx<'_>,
    v: &CourseVideo,
    id: &str,
    outdir: &std::path::Path,
) -> anyhow::Result<()> {
    println!(
        "下载课程回放：{} ({}, {})",
        v.course_name(),
        v.meta().title(),
        v.meta().time()
    );

    // prepare download dir
    let dir = utils::projectdir()
        .cache_dir()
        .join("video_download")
        .join(&id);
    fs::create_dir_all(&dir)
        .await
        .context("create dir failed")?;

    let paths = download_segments(ctx, &v, &dir)
        .await
        .context("download ts segments")?;

    let m3u8 = dir.join("playlist").with_extension("m3u8");
    buf_try!(@try fs::write(&m3u8, v.m3u8_raw()).await);

    // merge all segments into one file
    let merged = dir.join("merged").with_extension("ts");
    merge_segments(ctx, &merged, &paths).await?;

    let dest = outdir.join(video_output_filename(v));
    log::info!("Merged segments to {}", merged.display());
    log::info!(
        r#"You may execute `ffmpeg -i "{}" -c copy "{}"` to convert it to mp4"#,
        merged.display(),
        dest.display(),
    );

    // convert the merged ts file to mp4. overwrite existing file
    let sp = ctx.spinner();
    sp.set_message("Converting to mp4 file...");
    let c = compio::process::Command::new("ffmpeg")
        .args(["-y", "-hide_banner", "-loglevel", "quiet"])
        .args(["-i", merged.to_string_lossy().as_ref()])
        .args(["-c", "copy"])
        .arg(&dest)
        .output()
        .await
        .context("execute ffmpeg")?;
    ctx.remove_spinner(sp);

    if c.status.success() {
        println!(
            "下载完成, 文件保存为: {GR}{H2}{}{H2:#}{GR:#}",
            dest.display()
        );
    } else {
        anyhow::bail!("ffmpeg failed with exit code {:?}", c.status.code());
    }

    Ok(())
}

#[cfg(feature = "video-download")]
fn video_output_filename(v: &CourseVideo) -> String {
    let course = sanitize_filename_part(v.course_name());
    let time = sanitize_filename_part(v.meta().time());
    let title = sanitize_filename_part(v.meta().title());

    let stem = if title.is_empty() || title == time {
        format!("{course}_{time}")
    } else {
        format!("{course}_{time}_{title}")
    };
    format!("{stem}.mp4")
}

#[cfg(feature = "video-download")]
fn sanitize_filename_part(s: &str) -> String {
    let s = s
        .trim()
        .chars()
        .map(|c| match c {
            '<' | '>' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            ':' => '-',
            c if c.is_control() => '_',
            c => c,
        })
        .collect::<String>();

    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.is_empty() { "_".to_owned() } else { s }
}

#[cfg(feature = "video-download")]
async fn download_segments(
    ctx: &CommandCtx<'_>,
    v: &CourseVideo,
    dir: impl AsRef<std::path::Path>,
) -> anyhow::Result<Vec<std::path::PathBuf>> {
    let dir = dir.as_ref();
    if !dir.exists() {
        anyhow::bail!("dir {} not exists", dir.display());
    }

    let tot = v.len_segments();
    let pb = ctx.multi.add(pbar::new(tot as u64)).with_prefix("download");
    pb.tick();

    let mut key = None;
    let mut paths = Vec::new();
    // faster than try_join_all
    for i in 0..tot {
        key = v.refresh_key(i, key);
        let path = dir.join(&v.segment(i).uri).with_extension("ts");

        if !path.exists() {
            log::debug!("key: {key:?}");
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
    ctx.multi.remove(&pb);

    Ok(paths)
}

#[cfg(feature = "video-download")]
async fn merge_segments(
    ctx: &CommandCtx<'_>,
    dest: impl AsRef<std::path::Path>,
    paths: &[std::path::PathBuf],
) -> anyhow::Result<()> {
    let f = fs::File::create(&dest)
        .await
        .context("create merged file failed")?;
    let mut f = std::io::Cursor::new(f);

    let pb = ctx
        .multi
        .add(pbar::new(paths.len() as u64))
        .with_prefix("merge segments");
    pb.tick();
    for p in paths {
        let data = fs::read(p).await.context("read segments failed")?;
        buf_try!(@try f.write(data).await);
        pb.inc(1);
    }
    pb.finish_and_clear();
    ctx.multi.remove(&pb);

    Ok(())
}
