extern crate directories as dirs;

mod api;
mod cli;
mod config;
mod http;
mod multipart;
#[cfg(feature = "pdf")]
mod pdf;
mod qs;
#[cfg(feature = "ttshitu")]
mod ttshitu;
mod utils;
mod walkdir;

use shadow_rs::shadow;
shadow!(build);

use clap::Parser as _;

#[compio::main]
async fn main() {
    let logger = env_logger::Builder::new()
        .filter_level(log::LevelFilter::Warn)
        .parse_default_env()
        .filter_module("selectors::matching", log::LevelFilter::Info)
        .filter_module("html5ever::tokenizer", log::LevelFilter::Info)
        .filter_module("html5ever::tree_builder", log::LevelFilter::Error)
        .build();
    let level = logger.filter();
    let multi = indicatif::MultiProgress::new();
    indicatif_log_bridge::LogWrapper::new(multi.clone(), logger)
        .try_init()
        .unwrap();
    // ref: https://docs.rs/indicatif-log-bridge/latest/indicatif_log_bridge/#known-issues
    log::set_max_level(level);

    log::debug!("logger initialized...");

    let cli = cli::Cli::parse();

    match cli::start(cli, &multi).await {
        Ok(r) => r,
        Err(e) => {
            use utils::style::*;
            eprintln!("{RD}{B}Error{B:#}{RD:#}: {e:#}");
            std::process::exit(1);
        }
    }
}
