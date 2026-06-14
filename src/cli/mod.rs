mod cmd_announcement;
mod cmd_assignment;
#[cfg(feature = "bark")]
mod cmd_bark;
mod cmd_course_content;
mod cmd_course_table;
mod cmd_grades;
mod cmd_syllabus;
#[cfg(feature = "thesislib")]
mod cmd_thesis_lib;
#[cfg(feature = "ttshitu")]
mod cmd_ttshitu;
mod cmd_video;
mod pbar;

/// Shared CLI context (global progress group from [`main`]).
pub struct CommandCtx<'a> {
    pub multi: &'a MultiProgress,
    pub config_path: std::path::PathBuf,
}

impl CommandCtx<'_> {
    pub fn spinner(&self) -> AsyncSpinner {
        pbar::new_spinner_on(self.multi)
    }
    pub fn remove_spinner(&self, sp: AsyncSpinner) {
        self.multi.remove(&sp);
    }
}

use crate::api::{blackboard::*, syllabus::*};
use crate::cli::pbar::AsyncSpinner;
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
use indicatif::MultiProgress;
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
    long_about = "a Better BlackBoard for PKUers. 北京大学教学网命令行工具 (️Win/Linux/Mac), 支持查看/提交作业、查看公告、下载课程回放."
)]
pub struct Cli {
    /// 配置文件路径 (优先级高于 PKU3B_CONFIG)
    #[arg(long, global = true, env = "PKU3B_CONFIG", value_name = "PATH")]
    config: Option<std::path::PathBuf>,

    /// 缓存目录路径 (优先级高于 PKU3B_CACHE_DIR)
    #[arg(long, global = true, env = "PKU3B_CACHE_DIR", value_name = "PATH")]
    cache_dir: Option<std::path::PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// 获取课程作业信息/下载附件/提交作业
    #[command(visible_alias("a"), arg_required_else_help(true))]
    Assignment(cmd_assignment::CommandAssignment),

    /// 获取课程内容
    #[command(visible_alias("cc"))]
    CourseContent(cmd_course_content::CommandCourseContent),

    /// 获取个人课表
    #[command(name = "coursetable", visible_alias("ct"))]
    CourseTable(cmd_course_table::CommandCourseTable),

    /// 获取课程公告
    #[command(
        name = "announcement",
        visible_alias("ann"),
        arg_required_else_help(true)
    )]
    Announcement(cmd_announcement::CommandAnnouncement),

    /// 获取课程回放/下载课程回放
    #[command(visible_alias("v"), arg_required_else_help(true))]
    Video(cmd_video::CommandVideo),

    /// 选课操作
    #[command(visible_alias("s"), arg_required_else_help(true))]
    Syllabus(cmd_syllabus::CommandSyllabus),

    /// 图形验证码识别
    #[cfg(feature = "ttshitu")]
    #[command(visible_alias("tt"), arg_required_else_help(true))]
    Ttshitu(cmd_ttshitu::CommandTtshitu),

    /// Bark通知设置
    #[cfg(feature = "bark")]
    #[command(visible_alias("b"), arg_required_else_help(true))]
    Bark(cmd_bark::CommandBark),

    /// 查询课程成绩
    #[command(visible_alias("g"))]
    Grades(cmd_grades::CommandGrades),

    /// 学位论文检索
    #[cfg(feature = "thesislib")]
    #[command(visible_alias("th"), arg_required_else_help(true))]
    ThesisLib(cmd_thesis_lib::CommandThesisLib),

    /// (重新) 初始化用户名/密码
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
enum CacheCommands {
    /// 查看缓存大小
    Show,
    /// 清除缓存
    Clean,
}

impl clap::ValueEnum for DualDegree {
    fn value_variants<'a>() -> &'a [Self] {
        &[Self::Major, Self::Minor]
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        match self {
            Self::Major => Some(clap::builder::PossibleValue::new("major")),
            Self::Minor => Some(clap::builder::PossibleValue::new("minor")),
        }
    }
}

async fn build_client(enable_cache: bool) -> anyhow::Result<api::Client> {
    let mut builder =
        api::Client::builder().cookie_restore_path(Some(utils::default_user_agent_data_path()));
    if enable_cache {
        builder = builder
            .cache_ttl(Some(std::time::Duration::from_hours(1)))
            .download_artifact_ttl(Some(std::time::Duration::from_hours(24)))
    }
    builder.build().await
}

async fn load_blackboard(
    ctx: &CommandCtx<'_>,
    enable_cache: bool,
    otp_code: String,
) -> anyhow::Result<(Blackboard, AsyncSpinner)> {
    let sp = ctx.spinner();
    let client = build_client(enable_cache).await?;

    sp.set_message("reading config...");
    let cfg = config::read_cfg(&ctx.config_path)
        .await
        .context("read config file")?;

    let otp_code = if client
        .bb_login_require_otp(&cfg.username)
        .await
        .context("check if OTP is required")?
        && otp_code.is_empty()
    {
        inquire::Text::new("请输入手机令牌（OTP）码: ").prompt()?
    } else {
        otp_code
    };

    sp.set_message("logging in to blackboard...");
    let blackboard = client
        .blackboard(&cfg.username, &cfg.password, &otp_code)
        .await
        .context("login to blackboard")?;

    Ok((blackboard, sp))
}

/// Blackboard, courses and spinner are returned. Spinner hasn't stopped.
async fn load_client_courses(
    ctx: &CommandCtx<'_>,
    force: bool,
    only_current: bool,
    otp_code: String,
) -> anyhow::Result<(Blackboard, Vec<CourseHandle>, AsyncSpinner)> {
    let (b, sp) = load_blackboard(ctx, !force, otp_code).await?;

    sp.set_message("fetching courses...");
    let courses = b
        .get_courses(only_current)
        .await
        .context("fetch course handles")?;

    Ok((b, courses, sp))
}

async fn load_courses(
    ctx: &CommandCtx<'_>,
    force: bool,
    only_current: bool,
    otp_code: String,
) -> anyhow::Result<Vec<CourseHandle>> {
    let (_, r, _) = load_client_courses(ctx, force, only_current, otp_code).await?;
    Ok(r)
}

async fn command_config(
    ctx: &CommandCtx<'_>,
    attr: Option<config::ConfigAttrs>,
    value: Option<String>,
) -> anyhow::Result<()> {
    log::info!("Config path: '{}'", ctx.config_path.display());
    let mut cfg = match config::read_cfg(&ctx.config_path).await {
        Ok(r) => r,
        Err(e) => {
            anyhow::bail!("fail to read config: {e} (hint: run `pku3b init` to initialize it)")
        }
    };

    let Some(attr) = attr else {
        let s = toml::to_string_pretty(&cfg.redacted())?;
        println!("{s}");
        return Ok(());
    };

    if let Some(value) = value {
        cfg.update(attr, value)?;
        config::write_cfg(&ctx.config_path, &cfg).await?;
    } else {
        let mut buf = Vec::new();
        cfg.display(attr, &mut buf)?;
        buf_try!(@try fs::stdout().write_all(buf).await);
    }
    Ok(())
}

async fn command_init(ctx: &CommandCtx<'_>) -> anyhow::Result<()> {
    let username = inquire::Text::new("输入 PKU IAAA 学号:").prompt()?;
    let password = inquire::Text::new("输入 PKU IAAA 密码:").prompt()?;

    let cfg = config::Config {
        username,
        password,
        ttshitu: None,
        bark: None,
        secret_backend: config::SecretBackend::Plaintext,
        auto_supplement: None,
    };
    config::write_cfg(&ctx.config_path, &cfg).await?;

    println!("Configuration initialized.");
    Ok(())
}

async fn command_cache_clean(dry_run: bool) -> anyhow::Result<()> {
    let cache_dir = utils::cache_dir();
    log::info!("Cache dir: '{}'", cache_dir.display());
    let sp = pbar::new_spinner();
    sp.set_message("scanning cache dir...");

    let mut total_bytes = 0;
    if cache_dir.exists() {
        let d = std::fs::read_dir(&cache_dir)?;

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
            std::fs::remove_dir_all(&cache_dir)?;
        }
    }
    drop(sp);

    let sizenum = total_bytes as f64 / 1024.0f64.powi(3);
    if dry_run {
        println!("缓存大小: {B}{sizenum:.2}GB{B:#}");
    } else {
        println!("缓存已清空 (释放 {B}{sizenum:.2}GB{B:#})");
    }
    Ok(())
}

pub async fn start(cli: Cli, m: &MultiProgress) -> anyhow::Result<()> {
    if let Some(cache_dir) = cli.cache_dir.clone() {
        utils::set_cache_dir(cache_dir);
    }

    let config_path = cli
        .config
        .clone()
        .unwrap_or_else(utils::default_config_path);
    let ctx = CommandCtx {
        multi: m,
        config_path,
    };

    if let Some(command) = cli.command {
        match command {
            Commands::Config { attr, value } => command_config(&ctx, attr, value).await?,
            Commands::Init => command_init(&ctx).await?,
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
            Commands::Assignment(cmd) => cmd_assignment::run(cmd, &ctx).await?,
            Commands::CourseContent(cmd) => cmd_course_content::run(cmd, &ctx).await?,
            Commands::CourseTable(cmd) => cmd_course_table::run(cmd, &ctx).await?,
            Commands::Announcement(cmd) => cmd_announcement::run(cmd, &ctx).await?,
            Commands::Video(cmd) => cmd_video::run(cmd, &ctx).await?,
            Commands::Grades(cmd) => cmd_grades::run(cmd, &ctx).await?,
            Commands::Syllabus(cmd) => cmd_syllabus::run(cmd, &ctx).await?,

            #[cfg(feature = "ttshitu")]
            Commands::Ttshitu(cmd) => cmd_ttshitu::run(cmd, &ctx).await?,

            #[cfg(feature = "bark")]
            Commands::Bark(cmd) => cmd_bark::run(cmd, &ctx).await?,

            #[cfg(feature = "thesislib")]
            Commands::ThesisLib(cmd) => cmd_thesis_lib::run(cmd, &ctx).await?,

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
