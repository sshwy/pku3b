mod cmd_assignment;
mod cmd_video;
mod pbar;

use crate::{api, build, config, utils, walkdir};
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
use std::io::Write as _;
use utils::style::*;

#[derive(Parser)]
#[command(
    version,
    long_version(shadow_rs::formatcp!(
        "{}\nbuild_time: {}\nbuild_env: {}, {}\nbuild_target: {} (on {})",
        build::PKG_VERSION, build::BUILD_TIME, build::RUST_VERSION, build::RUST_CHANNEL,
        build::BUILD_TARGET, build::BUILD_OS
    )),
    author,
    about,
    long_about = "a Better BlackBoard for PKUers. 北京大学教学网命令行工具 (️Win/Linux/Mac), 支持查看/提交作业、下载课程回放."
)]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// 获取作业信息
    #[command(visible_alias("a"))]
    Assignment {
        /// 强制刷新
        #[arg(short, long, default_value = "false")]
        force: bool,

        #[command(subcommand)]
        command: Option<AssignmentCommands>,
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

#[derive(Subcommand)]
enum AssignmentCommands {
    /// 查看作业
    #[command(visible_alias("ls"))]
    List {
        /// 显示所有作业，包括已完成的
        #[arg(short, long, default_value = "false")]
        all: bool,
    },
    /// 下载作业要求和附件到指定文件夹下
    #[command(visible_alias("down"))]
    Download {
        /// 作业 ID (形如 `f4f30444c7485d49`, 可通过 `pku3b assignment list` 查看)
        id: String,
        /// 下载目录, 默认为当前目录
        #[arg(default_value = ".")]
        dir: std::path::PathBuf,
    },
    /// 提交作业
    #[command(visible_alias("sb"))]
    Submit {
        /// 作业 ID (形如 `f4f30444c7485d49`, 可通过 `pku3b assignment list` 查看)
        id: String,
        /// 提交文件
        path: std::path::PathBuf,
    },
}

async fn load_client_courses(
    force: bool,
) -> anyhow::Result<(api::Client, Vec<api::CourseHandle>, pbar::AsyncSpinner)> {
    let client = if force {
        api::Client::new_nocache()
    } else {
        api::Client::default()
    };

    let sp = pbar::new_spinner();

    sp.set_message("reading config...");
    let cfg_path = utils::default_config_path();
    let cfg = config::read_cfg(cfg_path)
        .await
        .context("read config file")?;

    sp.set_message("logging in to blackboard...");
    let blackboard = client
        .blackboard(&cfg.username, &cfg.password)
        .await
        .context("login to blackboard")?;

    sp.set_message("fetching courses...");
    let courses = blackboard
        .get_courses()
        .await
        .context("fetch course handles")?;

    Ok((client, courses, sp))
}

async fn load_courses(force: bool) -> anyhow::Result<Vec<api::CourseHandle>> {
    let (_, r, sp) = load_client_courses(force).await?;
    sp.finish_and_clear().await;
    Ok(r)
}

async fn command_config(
    attr: Option<config::ConfigAttrs>,
    value: Option<String>,
) -> anyhow::Result<()> {
    let cfg_path = utils::default_config_path();
    log::info!("Config path: '{}'", cfg_path.display());
    let mut cfg = match config::read_cfg(&cfg_path).await {
        Ok(r) => r,
        Err(e) => {
            anyhow::bail!("fail to read config: {e} (hint: run `pku3b init` to initialize it)")
        }
    };

    let Some(attr) = attr else {
        let s = toml::to_string_pretty(&cfg)?;
        println!("{}", s);
        return Ok(());
    };

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
        print!("{prompt}");
        std::io::stdout().flush().unwrap();
        let mut s = String::new();
        std::io::stdin().read_line(&mut s)?;
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
            #[cfg(unix)]
            let s = {
                use std::os::unix::fs::MetadataExt;
                e.metadata()?.size()
            };
            #[cfg(windows)]
            let s = {
                use std::os::windows::fs::MetadataExt;
                e.metadata()?.file_size()
            };
            total_bytes += s;
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
        println!("缓存已清空 (释放 {B}{:.2}GB{B:#})", sizenum);
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
            Commands::Assignment { force, command } => {
                if let Some(command) = command {
                    match command {
                        AssignmentCommands::List { all } => {
                            cmd_assignment::list(force, all).await?
                        }
                        AssignmentCommands::Download { id, dir } => {
                            cmd_assignment::download(&id, &dir).await?
                        }
                        AssignmentCommands::Submit { id, path } => {
                            cmd_assignment::submit(&id, &path).await?
                        }
                    }
                } else {
                    Cli::command()
                        .get_subcommands_mut()
                        .find(|s| s.get_name() == "assignment")
                        .unwrap()
                        .print_help()?;
                }
            }
            Commands::Video { force, command } => {
                if let Some(command) = command {
                    match command {
                        VideoCommands::List => cmd_video::list(force).await?,
                        VideoCommands::Download { id } => cmd_video::download(force, id).await?,
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

#[cfg(feature = "dev")]
async fn command_debug() -> anyhow::Result<()> {
    Ok(())
}
