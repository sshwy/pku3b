use std::io::Write as _;

use anyhow::Context as _;
use clap::{
    CommandFactory, Parser, Subcommand,
    builder::styling::{AnsiColor, Style},
};

use crate::{api, config, utils};

const ONE_HOUR: std::time::Duration = std::time::Duration::from_secs(3600);

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Fetch the latest data from the server
    Fetch {
        /// If specified, cache will be ignored
        #[arg(long, default_value = "false")]
        force: bool,
    },
    /// Reinitialize the configuration
    Init,
    /// Display or modify the configuration
    Config {
        // Path of the attribute to display or modify
        attr: config::ConfigAttrs,
        /// If specified, set the value of the attribute
        value: Option<String>,
    },
}

async fn command_config(attr: config::ConfigAttrs, value: Option<String>) -> anyhow::Result<()> {
    let cfg_path = utils::default_config_path();
    let mut cfg = config::read_cfg(&cfg_path)
        .await
        .context("read config file")?;
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

    let username = read_line("Username: ", false)?;
    let password = read_line("Password: ", true)?;

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

async fn command_fetch(force: bool) -> anyhow::Result<()> {
    let cfg_path = utils::default_config_path();
    let cfg = config::read_cfg(cfg_path)
        .await
        .context("read config file")?;

    let client = api::Client::new(if force { None } else { Some(ONE_HOUR) });
    eprintln!("Cache TTL: {:?}", client.cache_ttl());
    let blackboard = client.blackboard(&cfg.username, &cfg.password).await?;

    let courses = blackboard.get_courses().await?;
    // dbg!(&courses);

    for c in courses {
        let c = c.get().await?;
        let assignments = c.get_assignments().await?;
        // dbg!();
        let h1 = Style::new().bold().underline();
        let h2 = Style::new().underline();
        let h3 = Style::new().italic();
        let s = Style::new().dimmed();
        let gr = Style::new().fg_color(Some(AnsiColor::Green.into()));
        if !assignments.is_empty() {
            println!("{h1}[{}]{h1:#}\n", c.name());
            for a in assignments {
                let att = a.get_current_attempt().await?;
                let a = a.get().await?;
                if let Some(att) = att {
                    println!(
                        "{h2}{}{h2:#} ({gr}finished{gr:#}) {s}{att}{s:#}\n",
                        a.title()
                    );
                } else {
                    let t = a
                        .deadline()
                        .with_context(|| format!("fail to parse deadline: {}", a.deadline_raw()))?;
                    let delta = t - chrono::Local::now();
                    println!("{h2}{}{h2:#} ({})\n", a.title(), fmt_time_delta(delta),);
                }
                if !a.attachments().is_empty() {
                    println!("{h3}Attachments{h3:#}");
                    for (name, uri) in a.attachments() {
                        println!("{s}•{s:#} {}: {s}{}{s:#}", name, uri);
                    }
                    println!();
                }
                if !a.descriptions().is_empty() {
                    println!("{h3}Descriptions{h3:#}");
                    for p in a.descriptions() {
                        println!("{p}");
                    }
                }
                println!();
            }
        }
    }

    Ok(())
}

pub async fn start() -> anyhow::Result<()> {
    let cli = Cli::try_parse().context("parse CLI arguments")?;
    if let Some(command) = cli.command {
        match command {
            Commands::Config { attr, value } => command_config(attr, value).await?,
            Commands::Init => command_init().await?,
            Commands::Fetch { force } => command_fetch(force).await?,
        }
    } else {
        Cli::command().print_help()?;
    }

    Ok(())
}
