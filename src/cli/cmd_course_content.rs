use anyhow::Context;

use super::*;

#[derive(clap::Args)]
pub struct CommandCourseContent {
    /// 强制刷新
    #[arg(short, long, default_value = "false")]
    force: bool,

    /// 手机令牌码。当需要使用 OTP 登录，但未提供此参数时，将会从命令行交互式读取 OTP 码。
    #[arg(long, default_value = "")]
    otp_code: String,

    #[command(subcommand)]
    command: CourseContentCommands,
}

#[derive(Subcommand)]
enum CourseContentCommands {
    /// 查看每个课程的课程内容列表
    #[command(visible_alias("ls"))]
    List(ListOptions),

    /// 下载指定课程内容
    ///
    /// 使用 `list` 输出每行末尾的 `course_id:content_id` 定位条目（仅在该课程内查找）。
    /// 「文件」类型经教学网解析直链并以真实文件名保存；其余类型下载该条目下的全部附件。
    /// 可用 `--output-desc` 将描述另存为文本；未指定 `-o` 时保存到当前目录。
    #[command(visible_alias("down"))]
    Download(DownloadOptions),
}

#[derive(clap::Args)]
pub struct ListOptions {
    /// 将课程查询范围扩大到所有学期
    #[arg(long, default_value = "false")]
    all_term: bool,
    /// 指定课程标题的子串
    #[arg(long)]
    course_title: Option<String>,
}

pub async fn run(cmd: CommandCourseContent, ctx: &CommandCtx<'_>) -> anyhow::Result<()> {
    match cmd.command {
        CourseContentCommands::List(opts) => list(ctx, cmd.force, cmd.otp_code, opts).await?,
        CourseContentCommands::Download(opts) => {
            download(ctx, cmd.force, cmd.otp_code, opts).await?
        }
    }
    Ok(())
}

async fn get_contents(
    c: &Course,
    pb: indicatif::ProgressBar,
) -> anyhow::Result<Vec<CourseContent>> {
    let fut = async {
        let mut s = c.content_stream();

        // let pb = pbar::new(s.len() as u64).with_message("search contents");
        pb.set_length(s.len() as u64);
        pb.tick();

        let mut contents = Vec::new();
        while let Some(batch) = s.next_batch().await {
            contents.extend(batch);

            pb.set_length(s.len() as u64);
            pb.set_position(s.num_finished() as u64);
            pb.tick();
        }

        pb.finish_with_message("done.");
        Ok(contents)
    };

    let data = utils::with_cache(
        &format!("course_contents_of_{}", c.meta().id()),
        c.client().cache_ttl(),
        fut,
    )
    .await?;

    Ok(data.into_iter().map(|data| c.build_content(data)).collect())
}

async fn get_courses_contents(
    ctx: &CommandCtx<'_>,
    force: bool,
    otp_code: String,
    all_term: bool,
    course_title: Option<&str>,
    course_id: Option<&str>,
) -> anyhow::Result<Vec<(Course, Vec<CourseContent>)>> {
    let mut courses = load_courses(ctx, force, !all_term, otp_code).await?;

    if let Some(course_title) = course_title {
        log::debug!("filtering courses by title: {course_title}");
        courses.retain(|c| c.long_title().contains(course_title));
    }

    if let Some(course_id) = course_id {
        log::debug!("filtering courses by id: {course_id}");
        courses.retain(|c| c.id() == course_id);
    }

    // fetch each course concurrently
    let pb = ctx
        .multi
        .add(pbar::new(courses.len() as u64))
        .with_prefix("All");
    let futs = courses.into_iter().map(async |c| -> anyhow::Result<_> {
        let c = c.get().await.context("fetch course")?;
        let _pb = ctx
            .multi
            .add(pbar::new(0).with_prefix(c.meta().name().to_owned()));
        let contents = get_contents(&c, _pb)
            .await
            .with_context(|| format!("fetch assignment handles of {}", c.meta().title()))?;

        pb.inc(1);
        Ok((c, contents))
    });
    let courses = try_join_all(futs).await?;
    pb.finish_and_clear();
    ctx.multi.remove(&pb);

    Ok(courses)
}

pub async fn list(
    ctx: &CommandCtx<'_>,
    force: bool,
    otp_code: String,
    opts: ListOptions,
) -> anyhow::Result<()> {
    let courses = get_courses_contents(
        ctx,
        force,
        otp_code,
        opts.all_term,
        opts.course_title.as_deref(),
        None,
    )
    .await?;

    let mut outbuf = Vec::new();
    for (c, contents) in courses {
        writeln!(outbuf, "{BL}{B}{}{B:#}{BL:#}", c.meta().title())?;

        for ct in &contents {
            write!(
                outbuf,
                "{D}•{D:#} {MG}({:?}){MG:#} {}",
                ct.kind(),
                ct.title(),
            )?;
            if !ct.attachments().is_empty() {
                write!(outbuf, " [{B}{}{B:#} 附件]", ct.attachments().len())?;
            }
            writeln!(outbuf, " {D}{}{D:#}", ct.ccid())?;
        }
    }

    // write to stdout
    buf_try!(@try fs::stdout().write_all(outbuf).await);

    Ok(())
}

#[derive(clap::Args)]
pub struct DownloadOptions {
    /// 课程内容 ID（`course_id:content_id`，见 `pku3b cc list` 每行末尾）
    ccid: CourseContentID,
    /// 文件下载目录 (支持相对路径)
    #[arg(short = 'o', long)]
    outdir: Option<std::path::PathBuf>,
    /// 将课程内容描述写入文本文件，不指定则不写入
    #[arg(long)]
    output_desc: Option<String>,
    /// 将课程查询范围扩大到所有学期
    #[arg(long, default_value = "false")]
    all_term: bool,
    /// 指定课程标题的子串
    #[arg(long)]
    course_title: Option<String>,
}

pub async fn download(
    ctx: &CommandCtx<'_>,
    force: bool,
    otp_code: String,
    opts: DownloadOptions,
) -> anyhow::Result<()> {
    let courses = get_courses_contents(
        ctx,
        force,
        otp_code,
        opts.all_term,
        opts.course_title.as_deref(),
        Some(opts.ccid.course_id()),
    )
    .await?;

    let Some((c, ct)) = courses.into_iter().find_map(|(c, contents)| {
        contents
            .into_iter()
            .find(|ct| ct.ccid() == opts.ccid)
            .map(|ct| (c, ct))
    }) else {
        anyhow::bail!("course content with id {} not found", opts.ccid);
    };

    log::debug!("{:?}", ct);

    let outdir = opts.outdir.unwrap_or_else(|| std::path::PathBuf::from("."));
    fs::create_dir_all(&outdir).await?;

    println!("Content kind: {:?}", ct.kind());

    if let Some(output_desc) = &opts.output_desc {
        let dest = outdir.join(output_desc);
        println!("Writing description to {}", dest.display());
        buf_try!(@try fs::write(dest, ct.descriptions().join("\n")).await);
    }

    if matches!(ct.kind(), CourseContentKind::File) {
        let dest = outdir.join(ct.title());
        let uri = c
            .client()
            .bb_course_content_file_uri(ct.ccid().course_id(), ct.ccid().content_id())
            .await?;
        log::info!("File: {uri}");
        let filename = uri.rsplit_once('/').unwrap().1;
        let filename = percent_encoding::percent_decode(filename.as_bytes())
            .decode_utf8_lossy()
            .to_string();
        println!("Downloading file {filename} to {}", dest.display());
        let dest = outdir.join(&filename);
        c.client()
            .course_attachment_download(&uri, &dest, false)
            .await
            .with_context(|| format!("download attachment '{filename}'"))?;
    }

    let atts = ct.attachments();
    let tot = atts.len();
    if !atts.is_empty() {
        println!("Downloading {} attachments to {}", tot, outdir.display());

        let pb = ctx
            .multi
            .add(pbar::new(tot as u64))
            .with_prefix("download")
            .with_message(format!("[0/{tot}]"));
        pb.tick();

        for (i, (name, uri)) in atts.iter().enumerate() {
            let dest = outdir.join(name);
            pb.set_message(format!("[{}/{tot}] downloading '{name}'...", i + 1));
            log::info!("downloading attachment {name} to {}", dest.display());
            c.client()
                .course_attachment_download(uri, &dest, true)
                .await
                .with_context(|| format!("download attachment '{name}'"))?;
            pb.inc(1);
        }

        pb.finish_with_message("done.");
        ctx.multi.remove(&pb);
    }

    Ok(())
}
