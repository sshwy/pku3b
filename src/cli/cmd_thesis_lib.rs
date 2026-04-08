use std::sync::Arc;

use super::*;
use futures_util::stream;
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
    /// 下载学位论文并转换为 PDF 文件
    #[command(visible_alias("down"))]
    Download {
        /// 学位论文 ID (形如 `hU5a4no4tbM1zJIlMBTdy8YGcsX5YalOoY3wPzTosgs%3D`, 可通过 `pku3b th search [keyword]` 查看)
        keyid: String,
        /// 文件下载目录 (支持相对路径)
        #[arg(short = 'o', long)]
        outdir: Option<std::path::PathBuf>,
        /// 并发下载页数
        #[arg(short = 'j', long, default_value = "5")]
        job: u32,
        /// 是否保存每一页的图片
        #[arg(long)]
        save_image: bool,
    },
}

pub async fn run(cmd: CommandThesisLib) -> anyhow::Result<()> {
    match cmd.command {
        ThesisLibCommands::Search { keyword } => command_thesis_lib_search(keyword).await?,
        ThesisLibCommands::Download {
            keyid,
            outdir,
            job,
            save_image,
        } => command_thesis_lib_download(keyid, outdir, job, save_image).await?,
    }
    Ok(())
}

async fn command_thesis_lib_search(keyword: String) -> anyhow::Result<()> {
    let c = build_client(true).await?;

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
    n_job: u32,
    save_image: bool,
) -> anyhow::Result<()> {
    log::info!("save_image: {save_image}");
    #[cfg(not(feature = "thesislib-pdf"))]
    log::warn!("thesislib-pdf feature is not enabled, skipping pdf conversion");

    let c = build_client(true).await?;

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
    let doc = drm.get_pdf().await?;

    sp.finish_and_clear();
    drop(sp);

    let ids = (0..=doc.maxpage()).collect_vec();

    let m = indicatif::MultiProgress::new();
    let pb = m.add(pbar::new(ids.len() as u64)).with_prefix("PDF pages");

    let outdir = outdir.unwrap_or_else(|| std::path::PathBuf::from("."));
    fs::create_dir_all(&outdir).await?;
    let outdir = Arc::new(outdir);
    let doc = Arc::new(doc);

    let mut page_results = stream::iter(ids)
        .map(|id| {
            let doc = Arc::clone(&doc);
            let outdir = Arc::clone(&outdir);
            let pb = pb.clone();
            async move {
                let data = match doc.get_page_image(id).await {
                    Ok(i) => i,
                    Err(e) => {
                        pb.inc(1);
                        log::warn!("failed to fetch page image {id}: {e:#}");
                        return (id, None);
                    }
                };

                if save_image {
                    let p = outdir.join(format!("{id}.jpg"));
                    if let Err(e) = compio::fs::write(&p, data.clone()).await.0 {
                        log::warn!("save image of page {id}: {e}")
                    }
                }
                pb.inc(1);
                (id, Some(data))
            }
        })
        .buffer_unordered(n_job as usize)
        .collect::<Vec<_>>()
        .await;

    page_results.sort_by_key(|(id, _)| *id);
    let data: Vec<_> = page_results.into_iter().map(|(_, data)| data).collect();

    pb.finish();

    #[cfg(feature = "thesislib-pdf")]
    {
        log::info!("converting to pdf...");
        let pdf_path = outdir.join("output.pdf");
        crate::pdf::images2pdf(&data, pdf_path)?;
    }
    Ok(())
}
