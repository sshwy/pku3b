use anyhow::Context;

use super::*;

pub async fn show() -> anyhow::Result<()> {
    let c = api::Client::new_nocache();

    let sp = pbar::new_spinner();

    sp.set_message("reading config...");
    let cfg_path = utils::default_config_path();
    let cfg = config::read_cfg(cfg_path)
        .await
        .context("read config file")?;

    sp.set_message("logging in to syllabus...");
    let sy = c.syllabus(&cfg.username, &cfg.password).await?;

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
