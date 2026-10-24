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

    /// 下载课程回放视频 (MP4 格式)，HLS 分片支持断点续传
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
    }
    Ok(())
}

#[cfg(all(test, feature = "video-download"))]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};

    struct DownloadDir(std::path::PathBuf);

    impl DownloadDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "pku3b-mp4-{}-{}",
                std::process::id(),
                rand::random::<u64>()
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for DownloadDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    async fn response(status: &str, advertised: usize, body: &[u8]) -> cyper::Response {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let status = status.to_owned();
        let body = body.to_vec();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0; 1024];
            socket.read(&mut request).unwrap();
            write!(
                socket,
                "HTTP/1.1 {status}\r\nContent-Length: {advertised}\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            for chunk in body.chunks(4096) {
                socket.write_all(chunk).unwrap();
            }
        });
        let client = cyper::Client::builder().no_proxy().build().unwrap();
        let result = client
            .get(format!("http://{addr}/video.mp4"))
            .unwrap()
            .send()
            .await
            .unwrap();
        server.join().unwrap();
        result
    }

    #[compio::test]
    async fn publishes_complete_mp4_response() {
        let directory = DownloadDir::new();
        let dest = directory.0.join("video.mp4");
        let body = vec![42; 65536];
        let response = response("200 OK", body.len(), &body).await;
        download_mp4(response, &dest).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        assert!(!dest.with_extension("mp4.part").exists());
    }

    #[compio::test]
    async fn truncated_response_keeps_partial_and_existing_destination() {
        let directory = DownloadDir::new();
        let dest = directory.0.join("video.mp4");
        std::fs::write(&dest, b"existing video").unwrap();
        let response = response("200 OK", 1024, b"short body").await;
        assert!(download_mp4(response, &dest).await.is_err());
        assert_eq!(std::fs::read(&dest).unwrap(), b"existing video");
        assert!(dest.with_extension("mp4.part").exists());
    }

    #[compio::test]
    async fn rejects_http_errors_without_publishing_video() {
        let directory = DownloadDir::new();
        let dest = directory.0.join("video.mp4");
        let response = response("404 Not Found", 0, b"").await;
        assert!(download_mp4(response, &dest).await.is_err());
        assert!(!dest.exists());
        assert!(!dest.with_extension("mp4.part").exists());
    }

    #[compio::test]
    async fn rejects_empty_mp4_response() {
        let directory = DownloadDir::new();
        let dest = directory.0.join("video.mp4");
        let response = response("200 OK", 0, b"").await;
        assert!(download_mp4(response, &dest).await.is_err());
        assert!(!dest.exists());
    }
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
    let outdir = outdir.unwrap_or(std::path::Path::new("."));
    if !outdir.exists() {
        anyhow::bail!("output directory {:?} not exists", outdir.display());
    }

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
    let source = v.get().await?;

    ctx.remove_spinner(sp);

    let v = match source {
        CourseVideoSource::Hls(video) => video,
        CourseVideoSource::Mp4 {
            course_name,
            title,
            response,
        } => {
            println!("下载课程回放：{} ({})", course_name, title);
            let dest = outdir.join(format!("{}_{}.mp4", course_name, title));
            let sp = ctx.spinner();
            sp.set_message("Downloading MP4 file...");
            let result = download_mp4(response, &dest).await;
            ctx.remove_spinner(sp);
            result?;
            println!(
                "下载完成, 文件保存为: {GR}{H2}{}{H2:#}{GR:#}",
                dest.display()
            );
            return Ok(());
        }
    };

    println!("下载课程回放：{} ({})", v.course_name(), v.meta().title());

    // prepare download dir
    let dir = utils::cache_dir().join("video_download").join(&id);
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

    let dest = format!("{}_{}.mp4", v.course_name(), v.meta().title());
    let dest = outdir.join(&dest);
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
async fn download_mp4(response: cyper::Response, dest: &std::path::Path) -> anyhow::Result<()> {
    anyhow::ensure!(
        response.status() == http::StatusCode::OK,
        "get MP4 source returned {}",
        response.status()
    );
    let expected = response.content_length();
    let partial = dest.with_extension("mp4.part");
    let file = fs::File::create(&partial)
        .await
        .context("create MP4 partial file")?;
    let mut file = std::io::Cursor::new(file);
    let mut stream = response.bytes_stream();
    let mut downloaded = 0_u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("read MP4 response")?;
        downloaded += chunk.len() as u64;
        buf_try!(@try file.write_all(chunk).await);
    }
    anyhow::ensure!(downloaded > 0, "MP4 response was empty");
    if let Some(expected) = expected {
        anyhow::ensure!(downloaded == expected, "MP4 response length mismatch");
    }
    drop(file);
    fs::rename(&partial, dest)
        .await
        .context("publish downloaded MP4")?;
    Ok(())
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
