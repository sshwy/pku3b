use clap::Parser as _;

extern crate directories as dirs;

mod api;
mod cli;
mod config;
mod qs;
mod utils;

#[compio::main]
async fn main() {
    let cli = cli::Cli::parse();

    match cli::start(cli).await {
        Ok(r) => r,
        Err(e) => {
            let style = clap::builder::styling::Style::new()
                .fg_color(Some(clap::builder::styling::AnsiColor::Red.into()))
                .bold();
            eprintln!("{style}Error{style:#}: {e:#}");
            std::process::exit(1);
        }
    }
}
