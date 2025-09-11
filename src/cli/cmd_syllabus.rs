use anyhow::Context;

use crate::api::SyllabusSupplementCourseData;

use super::*;

pub async fn show(dual: Option<api::DualDegree>) -> anyhow::Result<()> {
    let c = api::Client::new_nocache();

    let sp = pbar::new_spinner();

    sp.set_message("reading config...");
    let cfg_path = utils::default_config_path();
    let cfg = config::read_cfg(cfg_path)
        .await
        .context("read config file")?;

    sp.set_message("logging in to syllabus...");
    let sy = c.syllabus(&cfg.username, &cfg.password, dual).await?;

    sp.set_message("fetching results...");
    let rs = sy.get_results().await?;

    drop(sp);

    for c in rs {
        let mut line = String::new();
        use std::fmt::Write;
        let st = if c.status == "已选上" { GR } else { RD };

        write!(
            line,
            "{st}{B}{}{B:#}  {st}{}{st:#}  {D}{}{D:#}  {}班 {BL}[{}]{BL:#}  {}",
            c.status, c.name, c.teacher, c.class_id, c.category, c.department
        )?;

        println!("{}", line);
    }

    Ok(())
}

pub async fn set_autoelective(dual: Option<api::DualDegree>) -> anyhow::Result<()> {
    let c = api::Client::new_nocache();

    let sp = pbar::new_spinner();

    sp.set_message("reading config...");
    let cfg_path = utils::default_config_path();
    let cfg = config::read_cfg(&cfg_path)
        .await
        .context("read config file")?;

    sp.set_message("logging in to syllabus...");
    let sy = c.syllabus(&cfg.username, &cfg.password, dual).await?;

    sp.set_message("fetching total pages...");
    let total = sy.get_supplements_total_and_elected().await?.0;

    let mut items = Vec::new();
    for i in 0..total {
        sp.set_message(format!("fetching page {}/{}...", i + 1, total));
        let data = sy.get_supplements(i).await?;
        items.extend(data.into_iter());
    }

    drop(sp);

    let c = select_supplement_course(items).await?;

    let mut cfg = cfg;
    let items = cfg.auto_supplement.get_or_insert_default();
    items.push(config::SupplementCourseConfig {
        page_id: c.page_id,
        name: c.base.name,
        teacher: c.base.teacher,
        class_id: c.base.class_id,
    });

    let items = items.to_owned();
    config::write_cfg(&cfg_path, &cfg).await?;

    println!("\n{GR}{B}添加成功{B:#}，您现在的补退选课程为：");
    for c in items {
        println!(
            "{D}P{}.{D:#} {B}{}{B:#}  {D}{}{D:#}  {}班",
            c.page_id + 1,
            c.name,
            c.teacher,
            c.class_id,
        );
    }

    Ok(())
}

async fn select_supplement_course(
    mut items: Vec<SyllabusSupplementCourseData>,
) -> anyhow::Result<SyllabusSupplementCourseData> {
    if items.is_empty() {
        anyhow::bail!("assignments not found");
    }

    let mut options = Vec::new();

    for c in items.iter() {
        options.push(supplement_course_desc(c));
    }

    let s = inquire::Select::new("请选择补退选课程", options).raw_prompt()?;
    let idx = s.index;
    let r = items.swap_remove(idx);

    Ok(r)
}

fn supplement_course_desc(c: &SyllabusSupplementCourseData) -> String {
    let mut line = String::new();
    use std::fmt::Write;

    write!(
        line,
        "P{}. {B}{}{B:#}  {D}{}{D:#}  {}班 {BL}[{}]{BL:#}  {} ({})",
        c.page_id + 1,
        c.name,
        c.teacher,
        c.class_id,
        c.category,
        c.department,
        c.status,
    )
    .unwrap();

    line
}

pub async fn unset_autoelective() -> anyhow::Result<()> {
    let sp = pbar::new_spinner();

    sp.set_message("reading config...");
    let cfg_path = utils::default_config_path();
    let mut cfg = config::read_cfg(&cfg_path)
        .await
        .context("read config file")?;

    drop(sp);

    let Some(items) = cfg.auto_supplement.as_mut() else {
        anyhow::bail!("您还没有设置自动补退选课程");
    };

    let idx = select_supplement_course_config(items).await?;

    items.remove(idx);

    config::write_cfg(&cfg_path, &cfg).await?;

    println!("{B}移除成功{B:#}");
    Ok(())
}

async fn select_supplement_course_config(
    items: &[config::SupplementCourseConfig],
) -> anyhow::Result<usize> {
    if items.is_empty() {
        anyhow::bail!("assignments not found");
    }

    let mut options = Vec::new();

    for c in items.iter() {
        options.push(format!(
            "P{}. {B}{}{B:#}  {D}{}{D:#}  {}班",
            c.page_id + 1,
            c.name,
            c.teacher,
            c.class_id,
        ));
    }

    let s = inquire::Select::new("请选择要移除的补退选课程", options).raw_prompt()?;
    let idx = s.index;
    Ok(idx)
}

#[cfg(feature = "autoelect")]
pub async fn launch_autoelective(
    interval: u64,
    dual: Option<api::DualDegree>,
) -> anyhow::Result<std::convert::Infallible> {
    let c = api::Client::new_nocache();

    let sp = pbar::new_spinner();

    sp.set_message("reading config...");
    let cfg_path = utils::default_config_path();
    let cfg = config::read_cfg(&cfg_path)
        .await
        .context("read config file")?;

    let Some(items) = cfg.auto_supplement else {
        anyhow::bail!("您还没有设置自动补退选课程");
    };
    if items.is_empty() {
        anyhow::bail!("您还没有设置自动补退选课程");
    }
    let Some(ttshitu) = &cfg.ttshitu else {
        anyhow::bail!("您还没有设置 TT 识图");
    };
    drop(sp);

    let sy_ctor = || c.syllabus(&cfg.username, &cfg.password, dual.clone());
    let items = items;

    Ok(autoelective_loop(sy_ctor, interval, &ttshitu, items, cfg.bark.as_ref()).await)
}

#[cfg(feature = "autoelect")]
async fn autoelective_loop<R: Future<Output = anyhow::Result<api::Syllabus>>>(
    sy_ctor: impl Fn() -> R,
    interval: u64,
    ttshitu: &config::TTShiTuConfig,
    items: Vec<config::SupplementCourseConfig>,
    bark_cfg: Option<&config::BarkConfig>,
) -> std::convert::Infallible {
    async fn run_loop(
        sy: &api::Syllabus,
        interval: u64,
        ttshitu: &config::TTShiTuConfig,
        #[allow(unused)] bark_cfg: Option<&config::BarkConfig>,
        mut items: Vec<config::SupplementCourseConfig>,
    ) -> anyhow::Result<std::convert::Infallible> {
        // Send the course selection start notification (optional)
        #[cfg(feature = "bark")]
        if let Some(bark) = bark_cfg {
            if let Err(e) = super::cmd_bark::send_bark_notification(
                &bark.token,
                "PKU3B 自动选课",
                &format!("开始监控 {} 门课程", items.len()),
            )
            .await
            {
                log::warn!("Bark 通知发送失败: {}", e);
            }
        }

        loop {
            let time = chrono::Local::now();
            println!("\n\n{BL}{B}共 {} 个课程{B:#} {D}{}{D:#}", items.len(), time);
            let (_, elected) = sy.get_supplements_total_and_elected().await?;
            let mut discards = Vec::new();

            for (cidx, c) in items.iter().enumerate() {
                if elected
                    .iter()
                    .any(|e| e.name == c.name && e.teacher == c.teacher && e.class_id == c.class_id)
                {
                    println!(
                        "{D}[{D:#}{}{D}/{D:#}{}{D}]{D:#} {GR}已选上课程 {B}{} {} {}班{B:#}",
                        cidx + 1,
                        items.len(),
                        c.name,
                        c.teacher,
                        c.class_id
                    );

                    // Send a successful course selection notification (optional)
                    #[cfg(feature = "bark")]
                    if let Some(bark) = bark_cfg {
                        if let Err(e) = super::cmd_bark::send_bark_notification(
                            &bark.token,
                            "PKU3B 选课成功",
                            &format!("已成功选上课程: {} {} {}班", c.name, c.teacher, c.class_id),
                        )
                        .await
                        {
                            log::warn!("Bark 通知发送失败: {}", e);
                        }
                    }

                    discards.push(cidx);
                    continue;
                }

                println!(
                    "{D}[{D:#}{}{D}/{D:#}{}{D}]{D:#} 查询课程 {B}{} {} {}班{B:#}...",
                    cidx + 1,
                    items.len(),
                    c.name,
                    c.teacher,
                    c.class_id
                );
                let data = sy.get_supplements(c.page_id).await?;
                let Some(index) = data.iter().position(|d| {
                    d.name == c.name && d.teacher == c.teacher && d.class_id == c.class_id
                }) else {
                    anyhow::bail!(
                        "课程 {} - {} - {} 在补退选列表中未找到",
                        c.name,
                        c.teacher,
                        c.class_id
                    );
                };

                let c = &data[index];

                if !c.is_full()? {
                    println!("{GR}有名额，正在尝试选课...{GR:#}");
                    sy.elect(c, ttshitu.username.clone(), ttshitu.password.clone())
                        .await?;
                }
            }

            discards.sort();
            discards.dedup();
            discards.reverse();
            for i in discards {
                items.remove(i);
            }

            println!("{D}等待 {} 秒后继续...{D:#}", interval);
            compio::time::sleep(std::time::Duration::from_secs(interval)).await;
        }
    }

    loop {
        let sy = loop {
            match sy_ctor().await {
                Ok(sy) => break sy,
                Err(e) => {
                    log::error!("登录选课网失败: {}", e);

                    // 发送登录失败通知（可选）
                    #[cfg(feature = "bark")]
                    if let Some(bark) = bark_cfg {
                        if let Err(bark_err) = super::cmd_bark::send_bark_notification(
                            &bark.token,
                            "PKU3B 选课登录失败",
                            &format!("登录选课网失败，正在重试: {}", e),
                        )
                        .await
                        {
                            log::warn!("Bark 通知发送失败: {}", bark_err);
                        }
                    }

                    println!("{D}等待 {} 秒后继续...{D:#}", interval);
                    compio::time::sleep(std::time::Duration::from_secs(interval)).await;
                }
            }
        };

        let Err(e) = run_loop(&sy, interval, ttshitu, bark_cfg, items.clone()).await;

        log::error!("自动选课循环异常: {}", e);
        log::warn!("正在重新登录...");

        // 发送选课循环异常通知（可选）
        #[cfg(feature = "bark")]
        if let Some(bark) = bark_cfg {
            if let Err(bark_err) = super::cmd_bark::send_bark_notification(
                &bark.token,
                "PKU3B 选课循环中断",
                &format!("选课循环出现异常，正在重新登录: {}", e),
            )
            .await
            {
                log::warn!("Bark 通知发送失败: {}", bark_err);
            }
        }
    }
}
