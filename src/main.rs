extern crate directories as dirs;

mod api;
mod cli;
mod config;
mod multipart;
mod qs;
mod utils;
mod walkdir;

use shadow_rs::shadow;
shadow!(build);

use clap::Parser as _;

#[compio::main]
async fn main() {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .unwrap();

    #[cfg(not(hyper_unstable_tracing))]
    {
        env_logger::builder()
            .filter_module("selectors::matching", log::LevelFilter::Info)
            .filter_module("html5ever::tokenizer", log::LevelFilter::Info)
            .filter_module("html5ever::tree_builder", log::LevelFilter::Info)
            .init();
    }

    #[cfg(hyper_unstable_tracing)]
    {
        tracing_subscriber::fmt::init();
    }

    log::debug!("logger initialized...");

    let cli = cli::Cli::parse();

    match cli::start(cli).await {
        Ok(r) => r,
        Err(e) => {
            use utils::style::*;
            eprintln!("{RD}{B}Error{B:#}{RD:#}: {e:#}");
            std::process::exit(1);
        }
    }
}
