use std::io::Write as _;

use anyhow::Context as _;
use clap::{CommandFactory, Parser, Subcommand, builder::styling::Style};

use crate::{api, config, utils};

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    // /// Optional name to operate on
    // name: Option<String>,

    // /// Sets a custom config file
    // #[arg(short, long, value_name = "FILE")]
    // config: Option<PathBuf>,

    // /// Turn debugging information on
    // #[arg(short, long, action = clap::ArgAction::Count)]
    // debug: u8,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    Run,
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

pub async fn start() -> anyhow::Result<()> {
    let cli = Cli::try_parse().context("parse CLI arguments")?;
    if let Some(command) = cli.command {
        match command {
            Commands::Config { attr, value } => command_config(attr, value).await?,
            Commands::Init => command_init().await?,
            Commands::Run => {
                let cfg_path = utils::default_config_path();
                let cfg = config::read_cfg(cfg_path)
                    .await
                    .context("read config file")?;

                let blackboard = api::Blackboard::oauth_login(&cfg.username, &cfg.password).await?;

                let courses = blackboard.get_courses().await?;
                // dbg!(&courses);

                for c in courses {
                    let c = c.get().await?;
                    let assignments = c.get_assignments().await?;
                    // dbg!();
                    if !assignments.is_empty() {
                        println!("Course: {}\n", c.name());
                        for a in assignments {
                            let att = a.get_current_attempt().await?;
                            let a = a.get().await?;
                            println!("{} [deadline: {}] {:?}\n", a.title(), a.deadline(), att);
                            if !a.attachments().is_empty() {
                                println!("Attachments:");
                                for (name, uri) in a.attachments() {
                                    println!("- {}: {}", name, uri);
                                }
                                println!();
                            }
                            if !a.descriptions().is_empty() {
                                println!("Descriptions:");
                                for p in a.descriptions() {
                                    println!("{p}");
                                }
                            }
                            println!();
                        }
                    }
                }
            }
        }
    } else {
        Cli::command().print_help()?;
    }

    Ok(())
}
