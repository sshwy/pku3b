mod pbar;

use crate::{api, config, utils, walkdir};
use anyhow::Context as _;
use clap::{
    CommandFactory, Parser, Subcommand,
    builder::styling::{AnsiColor, Style},
};
use compio::{
    buf::buf_try,
    fs,
    io::{AsyncWrite, AsyncWriteExt},
};
use futures_util::{StreamExt, future::try_join_all};
use std::{io::Write as _, os::unix::fs::MetadataExt};
use utils::style::*;

const ONE_HOUR: std::time::Duration = std::time::Duration::from_secs(3600);
const ONE_DAY: std::time::Duration = std::time::Duration::from_secs(3600 * 24);

#[derive(Parser)]
#[command(version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// 获取作业信息
    #[command(visible_alias("a"))]
    Assignment {
        /// 显示所有作业，包括已完成的
        #[arg(short, long, default_value = "false")]
        all: bool,

        /// 强制刷新
        #[arg(short, long, default_value = "false")]
        force: bool,
    },

    /// 获取课程回放/下载课程回放
    #[command(visible_alias("v"))]
    Video {
        /// 强制刷新
        #[arg(short, long, default_value = "false")]
        force: bool,

        #[command(subcommand)]
        command: Option<VideoCommands>,
    },

    /// (重新) 初始化配置选项
    Init,
    /// 显示或修改配置项
    Config {
        // 属性名称
        attr: Option<config::ConfigAttrs>,
        /// 属性值
        value: Option<String>,
    },
    /// 查看缓存大小/清除缓存
    Cache {
        #[command(subcommand)]
        command: Option<CacheCommands>,
    },

    #[cfg(feature = "dev")]
    #[command(hide(true))]
    Debug,
}

#[derive(Subcommand)]
enum VideoCommands {
    /// 获取课程回放列表
    #[command(visible_alias("ls"))]
    List,

    /// 下载课程回放视频 (MP4 格式)，支持断点续传
    #[command(visible_alias("down"))]
    Download {
        /// 课程回放 ID (形如 `e780808c9eb81f61`, 可通过 `pku3b video list` 查看)
        id: String,
    },
}

#[derive(Subcommand)]
enum CacheCommands {
    /// 查看缓存大小
    Show,

    /// 清除缓存
    Clean,
}

async fn command_config(
    attr: Option<config::ConfigAttrs>,
    value: Option<String>,
) -> anyhow::Result<()> {
    let cfg_path = utils::default_config_path();
    log::info!("Config path: '{}'", cfg_path.display());
    let cfg_res = config::read_cfg(&cfg_path).await;

    let Some(attr) = attr else {
        match cfg_res {
            Ok(cfg) => {
                let s = toml::to_string_pretty(&cfg)?;
                println!("{}", s);
            }
            Err(_) => {
                eprintln!("Fail to read config file. Run 'pku3b init' to initialize.");
            }
        }
        return Ok(());
    };

    let mut cfg = cfg_res.context("read config file")?;
    if let Some(value) = value {
        cfg.update(attr, value)?;
        config::write_cfg(&cfg_path, &cfg).await?;
    } else {
        let mut buf = Vec::new();
        cfg.display(attr, &mut buf)?;
        buf_try!(@try fs::stdout().write_all(buf).await);
    }
    Ok(())
}

async fn read_line(prompt: &str, is_password: bool) -> anyhow::Result<String> {
    if is_password {
        // use tricks to hide password
        let pass = rpassword::prompt_password(prompt.to_owned()).context("read password")?;
        Ok(pass)
    } else {
        buf_try!(@try fs::stdout().write_all(prompt.to_owned()).await);
        fs::stdout().flush().await?;
        let mut s = String::new();
        utils::stdin().read_line(&mut s).await?;
        Ok(s.trim().to_string())
    }
}

async fn command_init() -> anyhow::Result<()> {
    let cfg_path = utils::default_config_path();

    let username = read_line("PKU IAAA Username: ", false).await?;
    let password = read_line("PKU IAAA Password: ", true).await?;

    let cfg = config::Config { username, password };
    config::write_cfg(&cfg_path, &cfg).await?;

    println!("Configuration initialized.");
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

async fn command_fetch(force: bool, all: bool) -> anyhow::Result<()> {
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

async fn command_cache_clean(dry_run: bool) -> anyhow::Result<()> {
    let dir = utils::projectdir();
    log::info!("Cache dir: '{}'", dir.cache_dir().display());
    let sp = pbar::new_spinner();
    sp.set_message("scanning cache dir...");

    let mut total_bytes = 0;
    if dir.cache_dir().exists() {
        let d = std::fs::read_dir(dir.cache_dir())?;

        let mut s = walkdir::walkdir(d, false);
        while let Some(e) = s.next().await {
            let e = e?;
            total_bytes += e.metadata()?.size();
        }

        if !dry_run {
            std::fs::remove_dir_all(dir.cache_dir())?;
        }
    }
    sp.finish_and_clear().await;

    let sizenum = total_bytes as f64 / 1024.0f64.powi(3);
    if dry_run {
        println!("缓存大小: {B}{:.2}GB{B:#}", sizenum);
    } else {
        println!("缓存已清空 (释放 {B}{:.2}GB{B:#}GB)", sizenum);
    }
    Ok(())
}

async fn command_video_list(force: bool) -> anyhow::Result<()> {
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
    let blackboard = client.blackboard(&cfg.username, &cfg.password).await?;

    pb.set_message("fetching courses...");
    let courses = blackboard
        .get_courses()
        .await
        .context("fetch course handles")?;

    pb.finish_and_clear().await;

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

async fn command_video_download(force: bool, id: String) -> anyhow::Result<()> {
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
    let blackboard = client.blackboard(&cfg.username, &cfg.password).await?;

    pb.set_message("fetching courses...");
    let courses = blackboard
        .get_courses()
        .await
        .context("fetch course handles")?;

    pb.set_message("finding video...");
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
        pb.finish_and_clear().await;
        anyhow::bail!("video with id {} not found", id);
    };

    pb.set_message("fetch video metadata...");
    let v = v.get().await?;

    pb.finish_and_clear().await;

    println!("下载课程回放：{} ({})", v.course_name(), v.meta().title());

    // prepare download dir
    let dir = utils::projectdir()
        .cache_dir()
        .join("video_download")
        .join(&id);
    fs::create_dir_all(&dir)
        .await
        .context("create dir failed")?;

    let paths = download_segments(&v, &dir).await?;

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
    sp.finish_and_clear().await;

    if c.status.success() {
        println!("下载完成, 文件保存为: {GR}{H2}{}{H2:#}{GR:#}", dest);
    } else {
        anyhow::bail!("ffmpeg failed with exit code {:?}", c.status.code());
    }

    Ok(())
}

pub async fn start(cli: Cli) -> anyhow::Result<()> {
    if let Some(command) = cli.command {
        match command {
            Commands::Config { attr, value } => command_config(attr, value).await?,
            Commands::Init => command_init().await?,
            Commands::Cache { command } => {
                if let Some(command) = command {
                    match command {
                        CacheCommands::Clean => command_cache_clean(false).await?,
                        CacheCommands::Show => command_cache_clean(true).await?,
                    }
                } else {
                    command_cache_clean(true).await?
                }
            }
            Commands::Assignment { force, all } => command_fetch(force, all).await?,
            Commands::Video { force, command } => {
                if let Some(command) = command {
                    match command {
                        VideoCommands::List => command_video_list(force).await?,
                        VideoCommands::Download { id } => command_video_download(force, id).await?,
                    }
                } else {
                    Cli::command()
                        .get_subcommands_mut()
                        .find(|s| s.get_name() == "video")
                        .unwrap()
                        .print_help()?;
                }
            }

            #[cfg(feature = "dev")]
            Commands::Debug => command_debug().await?,
        }
    } else {
        Cli::command().print_help()?;
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
            let seg = v.get_segment_data(i, key).await?;

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

#[cfg(feature = "dev")]
async fn command_debug() -> anyhow::Result<()> {
    let id = "32fc3d139a4c22f7";

    let cfg_path = utils::default_config_path();
    let cfg = config::read_cfg(cfg_path)
        .await
        .context("read config file")?;

    let client = api::Client::new(Some(ONE_HOUR), Some(ONE_DAY));

    let blackboard = client.blackboard(&cfg.username, &cfg.password).await?;

    let courses = blackboard
        .get_courses()
        .await
        .context("fetch course handles")?;

    for c in courses {
        let c = c.get().await.context("fetch course")?;

        let vs = c.get_video_list().await?;
        for v in vs {
            if v.id() == id {
                let v = v.get().await?;
                let dir = utils::projectdir()
                    .cache_dir()
                    .join("video_download")
                    .join(id);
                fs::create_dir_all(&dir)
                    .await
                    .context("create dir failed")?;
                let paths = download_segments(&v, &dir).await?;

                let m3u8 = dir.join("playlist").with_extension("m3u8");
                buf_try!(@try fs::write(&m3u8, v.m3u8_raw()).await);

                // merge all segments into one file
                let merged = dir.join("merged").with_extension("ts");
                merge_segments(&merged, &paths).await?;
                println!("Merged segments to {}", merged.display());
                println!(
                    "You may use `ffmpeg -i {} -c copy output.mp4` to convert it to mp4",
                    merged.display()
                );
            }
        }
    }

    Ok(())
}
