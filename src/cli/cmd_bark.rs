use super::*;
use anyhow::Context;

pub async fn command_bark_init() -> anyhow::Result<()> {
    let cfg_path = utils::default_config_path();
    let mut cfg = config::read_cfg(&cfg_path)
        .await
        .context("read config file")?;

    let token = inquire::Text::new("输入 Bark 通知令牌:").prompt()?;

    cfg.bark = Some(config::BarkConfig { token });

    config::write_cfg(&cfg_path, &cfg).await?;

    println!("{GR}{B}Bark 通知令牌已更新{B:#}{GR:#}");
    Ok(())
}

pub async fn command_bark_test() -> anyhow::Result<()> {
    let cfg_path = utils::default_config_path();
    let cfg = config::read_cfg(&cfg_path)
        .await
        .context("read config file")?;

    let Some(bark_cfg) = cfg.bark.as_ref() else {
        anyhow::bail!("Bark 通知未配置，请先运行 'pku3b bark init'");
    };

    send_bark_notification(&bark_cfg.token, "PKU3B 测试", "Bark 通知功能正常").await?;

    println!("{GR}{B}Bark 通知发送成功{B:#}{GR:#}");
    Ok(())
}

pub async fn send_bark_notification(token: &str, title: &str, body: &str) -> anyhow::Result<()> {
    let client = cyper::Client::new();
    let url = format!(
        "https://api.day.app/{}/{}/{}",
        urlencoding::encode(token),
        urlencoding::encode(title),
        urlencoding::encode(body)
    );

    let response = client.get(url)?.send().await?;

    if response.status().is_success() {
        Ok(())
    } else {
        anyhow::bail!("Bark 通知发送失败: HTTP {}", response.status())
    }
}
