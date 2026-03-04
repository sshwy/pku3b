use std::sync::Arc;

use anyhow::Context;

use super::*;

type DocumentListItem = (Arc<api::Course>, String, api::CourseDocumentHandle);

async fn get_contents(
    c: &api::Course,
    pb: indicatif::ProgressBar,
) -> anyhow::Result<Vec<api::CourseContent>> {
    let fut = async {
        let mut s = c.content_stream();

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
        &format!("get_course_documents_{}", c.meta().id()),
        c.client().cache_ttl(),
        fut,
    )
    .await?;

    Ok(data.into_iter().map(|data| c.build_content(data)).collect())
}

async fn get_documents(
    c: &api::Course,
    pb: indicatif::ProgressBar,
) -> anyhow::Result<Vec<api::CourseDocumentHandle>> {
    let r = get_contents(c, pb)
        .await?
        .into_iter()
        .filter_map(|c| c.into_document_opt())
        .collect();
    Ok(r)
}

async fn get_courses_and_documents(
    force: bool,
    cur_term: bool,
) -> anyhow::Result<Vec<(api::Course, Vec<api::CourseDocumentHandle>)>> {
    let courses = load_courses(force, cur_term).await?;

    // fetch each course concurrently
    let m = indicatif::MultiProgress::new();
    let pb = m.add(pbar::new(courses.len() as u64)).with_prefix("All");
    let futs = courses.into_iter().map(async |c| -> anyhow::Result<_> {
        let c = c.get().await.context("fetch course")?;
        let documents = get_documents(
            &c,
            m.add(pbar::new(0).with_prefix(c.meta().name().to_owned())),
        )
        .await
        .with_context(|| format!("fetch document handles of {}", c.meta().title()))?;

        pb.inc_length(documents.len() as u64);
        let futs = documents.into_iter().map(async |d| -> anyhow::Result<_> {
            pb.inc(1);
            Ok(d)
        });
        let documents = try_join_all(futs).await?;

        pb.inc(1);
        Ok((c, documents))
    });
    let courses = try_join_all(futs).await?;
    pb.finish_and_clear();
    m.clear().unwrap();
    drop(pb);
    drop(m);

    Ok(courses)
}

pub async fn list(force: bool, _all: bool, cur_term: bool) -> anyhow::Result<()> {
    let courses = get_courses_and_documents(force, cur_term).await?;

    let all_documents = courses
        .iter()
        .flat_map(|(c, documents)| {
            documents
                .iter()
                .map(move |d| (c.to_owned(), d.id().to_owned(), d.clone()))
        })
        .collect::<Vec<_>>();

    let mut sorted_documents = all_documents;

    // sort by title
    log::debug!("sorting documents...");
    sorted_documents.sort_by_cached_key(|(_, _, d)| d.title().to_string());

    // prepare output statements
    let mut outbuf = Vec::new();
    let title = "课程文档/课件";
    let total = sorted_documents.len();
    writeln!(outbuf, "{D}>{D:#} {B}{title} ({total}){B:#} {D}<{D:#}\n")?;

    for (c, id, d) in sorted_documents {
        write_document_title(&mut outbuf, &id, &c, &d).context("io error")?;
    }

    // write to stdout
    buf_try!(@try fs::stdout().write_all(outbuf).await);

    Ok(())
}

fn write_document_title(
    buf: &mut Vec<u8>,
    id: &str,
    c: &api::Course,
    d: &api::CourseDocumentHandle,
) -> std::io::Result<()> {
    use utils::style::*;
    write!(buf, "{BL}{B}{}{B:#}{BL:#} {D}>{D:#} ", c.meta().name())?;
    write!(buf, "{BL}{B}{}{B:#}{BL:#}", d.title())?;
    let att_count = d.attachments().len();
    if att_count > 0 {
        write!(buf, " ({GR}{att_count} 个附件{GR:#})")?;
    }
    writeln!(buf, " {D}{id}{D:#}")?;

    if !d.descriptions().is_empty() {
        writeln!(buf)?;
        for p in d.descriptions() {
            writeln!(buf, "{p}")?;
        }
    }
    if !d.attachments().is_empty() {
        writeln!(buf)?;
        for (name, _) in d.attachments() {
            writeln!(buf, "{D}[附件]{D:#} {UL}{name}{UL:#}")?;
        }
    }
    writeln!(buf)?;

    Ok(())
}

async fn fetch_documents(
    force: bool,
    cur_term: bool,
) -> anyhow::Result<Vec<DocumentListItem>> {
    let courses = get_courses_and_documents(force, cur_term).await?;

    let mut all_documents = courses
        .into_iter()
        .flat_map(|(c, documents)| {
            let c = Arc::new(c);
            documents
                .into_iter()
                .map(move |d| (c.clone(), d.id().to_owned(), d))
        })
        .collect::<Vec<_>>();

    // sort by title
    log::debug!("sorting documents...");
    all_documents.sort_by_cached_key(|(_, _, d)| d.title().to_string());

    Ok(all_documents)
}

async fn select_document(
    mut items: Vec<DocumentListItem>,
) -> anyhow::Result<DocumentListItem> {
    if items.is_empty() {
        anyhow::bail!("documents not found");
    }

    let mut options = Vec::new();

    for (idx, (c, id, d)) in items.iter().enumerate() {
        let mut outbuf = Vec::new();
        write!(outbuf, "[{}] ", idx + 1)?;
        write_document_title(&mut outbuf, id, c, d).context("io error")?;
        options.push(String::from_utf8(outbuf).unwrap());
    }

    let s = inquire::Select::new("请选择要下载的文档", options).raw_prompt()?;
    let idx = s.index;
    let r = items.swap_remove(idx);

    Ok(r)
}

pub async fn download(
    id: Option<&str>,
    dir: &std::path::Path,
    force: bool,
    cur_term: bool,
) -> anyhow::Result<()> {
    let items = fetch_documents(force, cur_term).await?;
    let d = match id {
        Some(id) => match items.into_iter().find(|x| x.1 == id) {
            Some(r) => r,
            None => anyhow::bail!("document with id {} not found", id),
        },
        None => select_document(items).await?,
    };

    let sp = pbar::new_spinner();
    download_data(sp, dir, &d.2).await?;

    Ok(())
}

async fn download_data(
    sp: pbar::AsyncSpinner,
    dir: &std::path::Path,
    d: &api::CourseDocumentHandle,
) -> anyhow::Result<()> {
    if !dir.exists() {
        compio::fs::create_dir_all(dir).await?;
    }

    let atts = d.attachments();
    let tot = atts.len();
    if tot == 0 {
        anyhow::bail!("no attachments to download");
    }

    for (idx, (name, uri)) in atts.iter().enumerate() {
        sp.set_message(format!(
            "[{}/{tot}] downloading attachment '{name}'...",
            idx + 1
        ));
        d.download_attachment(uri, &dir.join(name))
            .await
            .with_context(|| format!("download attachment '{name}'"))?;
    }

    drop(sp);
    println!("Done.");
    Ok(())
}
