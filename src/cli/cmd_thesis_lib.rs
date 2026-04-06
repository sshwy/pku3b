use super::*;
use itertools::Itertools;

#[derive(clap::Args)]
pub struct CommandThesisLib {
    #[command(subcommand)]
    command: ThesisLibCommands,
}

#[derive(Subcommand)]
enum ThesisLibCommands {
    /// 搜索学位论文
    Search { keyword: String },
    /// 下载课程回放视频 (MP4 格式)，支持断点续传
    #[command(visible_alias("down"))]
    Download {
        /// 学位论文 ID (形如 `hU5a4no4tbM1zJIlMBTdy8YGcsX5YalOoY3wPzTosgs%3D`, 可通过 `pku3b th search [keyword]` 查看)
        keyid: String,
        /// 文件下载目录 (支持相对路径)
        #[arg(short = 'o', long)]
        outdir: Option<std::path::PathBuf>,
    },
}

pub async fn run(cmd: CommandThesisLib) -> anyhow::Result<()> {
    match cmd.command {
        ThesisLibCommands::Search { keyword } => command_thesis_lib_search(keyword).await?,
        ThesisLibCommands::Download { keyid, outdir } => {
            command_thesis_lib_download(keyid, outdir).await?
        }
    }
    Ok(())
}

async fn command_thesis_lib_search(keyword: String) -> anyhow::Result<()> {
    let c = api::Client::default();

    let sp = pbar::new_spinner();

    sp.set_message("reading config...");
    let cfg_path = utils::default_config_path();
    let cfg = config::read_cfg(cfg_path)
        .await
        .context("read config file")?;

    sp.set_message("logging in to thesis.lib.pku.edu.cn...");
    let c = c.thesis_lib(&cfg.username, &cfg.password).await?;

    sp.set_message("searching for papers...");
    let data = c.search(&keyword).await?;

    sp.finish_and_clear();

    let mut buf = Vec::new();
    for item in data.items() {
        writeln!(
            buf,
            "{B}{}{B:#}  {} {} {} {} {}  {D}{}{D:#}",
            item.title,
            item.author,
            item.department,
            item.teacher_name,
            item.degree_type,
            item.degree_year,
            item.keyid
        )?;
    }

    // write to stdout
    buf_try!(@try fs::stdout().write_all(buf).await);

    Ok(())
}

async fn command_thesis_lib_download(
    keyid: String,
    outdir: Option<std::path::PathBuf>,
) -> anyhow::Result<()> {
    #[cfg(not(feature = "thesislib-pdf"))]
    log::warn!("thesislib-pdf feature is not enabled, skipping pdf conversion");

    let c = api::Client::default();

    let sp = pbar::new_spinner();

    sp.set_message("reading config...");
    let cfg_path = utils::default_config_path();
    let cfg = config::read_cfg(cfg_path)
        .await
        .context("read config file")?;

    sp.set_message("logging in to thesis.lib.pku.edu.cn...");
    let c = c.thesis_lib(&cfg.username, &cfg.password).await?;

    sp.set_message("fetching drm view...");
    let drm = c.drm_view(&keyid).await?;

    sp.set_message("fetching pdf document index...");
    let mut doc = drm.get_pdf().await?;

    sp.finish_and_clear();
    drop(sp);

    let ids = (0..=doc.maxpage()).collect_vec();
    let mut paths = vec![];

    let m = indicatif::MultiProgress::new();
    let pb = m.add(pbar::new(ids.len() as u64)).with_prefix("PDF pages");

    let outdir = outdir.unwrap_or_else(|| std::path::PathBuf::from("."));
    fs::create_dir_all(&outdir).await?;

    for id in ids {
        let data = match doc.get_page_image(id).await {
            Ok(i) => i,
            Err(e) => {
                pb.inc(1);
                log::warn!("failed to fetch page image {id}: {e:#}");
                continue;
            }
        };

        let p = outdir.join(format!("{id}.jpg"));
        compio::buf::buf_try!(@try compio::fs::write(&p, data).await);
        paths.push(p);
        pb.inc(1);
    }

    pb.finish();

    #[cfg(feature = "thesislib-pdf")]
    {
        log::info!("converting to pdf...");
        let pdf_path = outdir.join("output.pdf");
        crate::pdf::images2pdf(&paths, pdf_path)?;
    }
    Ok(())
}
