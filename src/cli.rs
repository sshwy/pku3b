use crate::{api, config, utils};
use anyhow::Context as _;
use clap::{
    CommandFactory, Parser, Subcommand,
    builder::styling::{AnsiColor, Style},
};
use std::io::Write as _;
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

    /// 获取课程回放
    #[command(visible_alias("v"))]
    Video,

    /// (重新) 初始化配置选项
    Init,
    /// 显示或修改配置项
    Config {
        // 属性名称
        attr: Option<config::ConfigAttrs>,
        /// 属性值
        value: Option<String>,
    },
    /// 清除缓存
    Clean,
}

async fn command_config(
    attr: Option<config::ConfigAttrs>,
    value: Option<String>,
) -> anyhow::Result<()> {
    let cfg_path = utils::default_config_path();
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
        cfg.display(attr, &mut std::io::stdout())?;
    }
    Ok(())
}

fn read_line(prompt: &str, is_password: bool) -> anyhow::Result<String> {
    if is_password {
        // use tricks to hide password
        let pass = rpassword::prompt_password(prompt.to_owned()).context("read password")?;
        Ok(pass)
    } else {
        print!("{}", prompt);
        let _ = std::io::stdout().flush();
        let mut s = String::new();
        std::io::stdin().read_line(&mut s)?;
        Ok(s.trim().to_string())
    }
}

async fn command_init() -> anyhow::Result<()> {
    let cfg_path = utils::default_config_path();
    let style = Style::new().underline();

    eprintln!("Config path: '{style}{}{style:#}'", cfg_path.display());

    let username = read_line("PKU IAAA Username: ", false)?;
    let password = read_line("PKU IAAA Password: ", true)?;

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

fn write_course_assignments(
    writer: &mut impl std::io::Write,
    a: &api::CourseAssignment,
) -> anyhow::Result<()> {
    if let Some(att) = a.last_attempt() {
        writeln!(
            writer,
            "{MG}{H2}{}{H2:#}{MG:#} ({GR}已完成{GR:#}) {D}{att}{D:#}\n",
            a.title()
        )?;
    } else {
        let t = a
            .deadline()
            .with_context(|| format!("fail to parse deadline: {}", a.deadline_raw()))?;
        let delta = t - chrono::Local::now();
        writeln!(
            writer,
            "{MG}{H2}{}{H2:#}{MG:#} ({})\n",
            a.title(),
            fmt_time_delta(delta),
        )?;
    }
    if !a.attachments().is_empty() {
        writeln!(writer, "{H3}附件{H3:#}")?;
        for (name, uri) in a.attachments() {
            writeln!(writer, "{D}•{D:#} {name}: {D}{uri}{D:#}")?;
        }
        writeln!(writer,)?;
    }
    if !a.descriptions().is_empty() {
        writeln!(writer, "{H3}描述{H3:#}")?;
        for p in a.descriptions() {
            writeln!(writer, "{p}")?;
        }
    }
    writeln!(writer)?;

    Ok(())
}

async fn command_fetch(force: bool, all: bool) -> anyhow::Result<()> {
    let cfg_path = utils::default_config_path();
    let cfg = config::read_cfg(cfg_path)
        .await
        .context("read config file")?;

    let mut outbuf = Vec::new();
    let title = if all {
        "所有作业 (包括已完成)"
    } else {
        "未完成作业"
    };

    writeln!(outbuf, "{D}>{D:#} {B}{}{B:#} {D}<{D:#}\n", title)?;

    let client = api::Client::new(
        if force { None } else { Some(ONE_HOUR) },
        if force { None } else { Some(ONE_DAY) },
    );
    let blackboard = client.blackboard(&cfg.username, &cfg.password).await?;

    let courses = blackboard
        .get_courses()
        .await
        .context("fetch course handles")?;

    // fetch each course and prepare output statements
    for c in courses {
        let c = c.get().await.context("fetch course")?;
        let assignments = c
            .get_assignments()
            .await
            .with_context(|| format!("fetch assignment handles of {}", c.name()))?;

        if assignments.is_empty() {
            continue;
        }

        writeln!(outbuf, "{BL}{H1}[{}]{H1:#}{BL:#}\n", c.name())?;
        for a in assignments {
            let a = a.get().await.context("fetch assignment")?;

            // skip finished assignments if not in full mode
            if a.last_attempt().is_some() && !all {
                continue;
            }

            write_course_assignments(&mut outbuf, &a)?;
        }
    }

    // write to stdout
    std::io::stdout().write_all(&outbuf)?;

    Ok(())
}

async fn command_clean() -> anyhow::Result<()> {
    let dir = utils::projectdir();
    if dir.cache_dir().exists() {
        std::fs::remove_dir_all(dir.cache_dir())?;
    }
    println!("Cache cleaned.");
    Ok(())
}

async fn command_video() -> anyhow::Result<()> {
    let cfg_path = utils::default_config_path();
    let cfg = config::read_cfg(cfg_path)
        .await
        .context("read config file")?;

    let mut outbuf = Vec::new();
    let title = "课程回放";

    writeln!(outbuf, "{D}>{D:#} {B}{}{B:#} {D}<{D:#}\n", title)?;

    let client = api::Client::new(Some(ONE_HOUR), Some(ONE_DAY));

    let blackboard = client.blackboard(&cfg.username, &cfg.password).await?;

    let courses = blackboard
        .get_courses()
        .await
        .context("fetch course handles")?;

    for c in courses {
        let c = c.get().await.context("fetch course")?;
        let vs = c.get_video_list().await.context("fetch video list")?;

        if vs.is_empty() {
            continue;
        }

        writeln!(outbuf, "{BL}{H1}[{}]{H1:#}{BL:#}\n", c.name())?;

        for v in vs {
            let v = v.get().await.context("fetch video")?;
            writeln!(outbuf, "{D}•{D:#} {} ({})", v.title(), v.time())?;
        }

        writeln!(outbuf)?;
    }

    std::io::stdout().write_all(&outbuf)?;
    Ok(())
}

pub async fn start(cli: Cli) -> anyhow::Result<()> {
    if let Some(command) = cli.command {
        match command {
            Commands::Config { attr, value } => command_config(attr, value).await?,
            Commands::Init => command_init().await?,
            Commands::Clean => command_clean().await?,
            Commands::Assignment { force, all } => command_fetch(force, all).await?,
            Commands::Video => command_video().await?,
        }
    } else {
        Cli::command().print_help()?;
    }

    Ok(())
}
