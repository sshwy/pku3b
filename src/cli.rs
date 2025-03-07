use std::io::Write as _;

use anyhow::Context as _;
use clap::{
    CommandFactory, Parser, Subcommand,
    builder::styling::{AnsiColor, Style},
};

use crate::{api, config, utils};

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
    /// Fetch the latest data from course.pku.edu.cn
    Fetch {
        /// Display all information
        #[arg(long, default_value = "false")]
        all: bool,

        /// If specified, cache will be ignored
        #[arg(long, default_value = "false")]
        force: bool,
    },
    /// (Re)initialize the configuration
    Init,
    /// Display or modify the configuration
    Config {
        // Path of the attribute to display or modify
        attr: Option<config::ConfigAttrs>,
        /// If specified, set the value of the attribute
        value: Option<String>,
    },
    /// Clean the cache
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

fn print_course_assignments(a: &api::CourseAssignments) -> anyhow::Result<()> {
    use utils::style::*;

    if let Some(att) = a.last_attempt() {
        println!(
            "{MG}{H2}{}{H2:#}{MG:#} ({GR}finished{GR:#}) {D}{att}{D:#}\n",
            a.title()
        );
    } else {
        let t = a
            .deadline()
            .with_context(|| format!("fail to parse deadline: {}", a.deadline_raw()))?;
        let delta = t - chrono::Local::now();
        println!(
            "{MG}{H2}{}{H2:#}{MG:#} ({})\n",
            a.title(),
            fmt_time_delta(delta),
        );
    }
    if !a.attachments().is_empty() {
        println!("{H3}Attachments{H3:#}");
        for (name, uri) in a.attachments() {
            println!("{D}•{D:#} {name}: {D}{uri}{D:#}");
        }
        println!();
    }
    if !a.descriptions().is_empty() {
        println!("{H3}Descriptions{H3:#}");
        for p in a.descriptions() {
            println!("{p}");
        }
    }
    println!();

    Ok(())
}

async fn command_fetch(force: bool, all: bool) -> anyhow::Result<()> {
    println!("Fetching Courses...");
    use utils::style::*;

    let cfg_path = utils::default_config_path();
    let cfg = config::read_cfg(cfg_path)
        .await
        .context("read config file")?;

    let client = api::Client::new(
        if force { None } else { Some(ONE_HOUR) },
        if force { None } else { Some(ONE_DAY) },
    );
    // eprintln!("Cache TTL: {:?}", client.cache_ttl());
    let blackboard = client.blackboard(&cfg.username, &cfg.password).await?;

    let courses = blackboard
        .get_courses()
        .await
        .context("fetch course handles")?;

    for c in courses {
        let c = c.get().await.context("fetch course")?;
        let assignments = c
            .get_assignments()
            .await
            .with_context(|| format!("fetch assignment handles of {}", c.name()))?;

        if !assignments.is_empty() {
            println!("{BL}{H1}[{}]{H1:#}{BL:#}\n", c.name());
            for a in assignments {
                let a = a.get().await.context("fetch assignment")?;

                // skip finished assignments if not in full mode
                if a.last_attempt().is_some() && !all {
                    continue;
                }

                print_course_assignments(&a)?;
            }
        }
    }

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

pub async fn start(cli: Cli) -> anyhow::Result<()> {
    if let Some(command) = cli.command {
        match command {
            Commands::Config { attr, value } => command_config(attr, value).await?,
            Commands::Init => command_init().await?,
            Commands::Fetch { force, all } => command_fetch(force, all).await?,
            Commands::Clean => command_clean().await?,
        }
    } else {
        Cli::command().print_help()?;
    }

    Ok(())
}
