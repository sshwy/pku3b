use compio::{buf::buf_try, fs, io::AsyncReadAtExt};

pub mod style {
    use clap::builder::styling::{AnsiColor, Color, Style};

    pub const D: Style = Style::new().dimmed();
    pub const B: Style = Style::new().bold();
    pub const H1: Style = Style::new().bold().underline();
    pub const H2: Style = UL;
    pub const UL: Style = Style::new().underline();
    pub const EM: Style = Style::new().italic();
    pub const GR: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::Green)));
    pub const MG: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::BrightMagenta)));
    pub const BL: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::Cyan)));
    pub const RD: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::Red)));
}

pub fn projectdir() -> dirs::ProjectDirs {
    dirs::ProjectDirs::from("org", "sshwy", "pku3b").expect("could not find project directories")
}

pub fn default_config_path() -> std::path::PathBuf {
    crate::utils::projectdir().config_dir().join("cfg.toml")
}

/// If the cache file exists and is not expired, return the deserialized content.
/// Otherwise, execute the future, serialize the result to the cache file, and return the result.
pub async fn with_cache<T, F>(
    name: &str,
    ttl: Option<&std::time::Duration>,
    fut: F,
) -> anyhow::Result<T>
where
    F: std::future::Future<Output = anyhow::Result<T>>,
    T: serde::de::DeserializeOwned + serde::Serialize + 'static,
{
    let name_hash = {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        name.hash(&mut hasher);
        let type_id = std::any::TypeId::of::<T>();
        type_id.hash(&mut hasher);
        hasher.finish()
    };
    let name = format!("with_cache-{:x}", name_hash);

    let path = &projectdir().cache_dir().join(&name);

    if let Ok(f) = fs::File::open(path).await {
        if let Some(ttl) = ttl {
            if f.metadata().await?.modified()?.elapsed()? < *ttl {
                let r = f.read_to_end_at(Vec::new(), 0).await;
                let (_, buf) = buf_try!(@try r);
                // ignore deserialization error
                if let Ok(r) = serde_json::from_slice(&buf) {
                    log::trace!("cache hit: {}", name);
                    return Ok(r);
                }
            }
        }
    }

    let r = fut.await?;
    fs::create_dir_all(path.parent().unwrap()).await?;
    let buf = serde_json::to_vec(&r)?;
    buf_try!(@try fs::write(path, buf).await);

    Ok(r)
}

pub async fn with_cache_bytes<F>(
    name: &str,
    ttl: Option<&std::time::Duration>,
    fut: F,
) -> anyhow::Result<bytes::Bytes>
where
    F: std::future::Future<Output = anyhow::Result<bytes::Bytes>>,
{
    let name_hash = {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        name.hash(&mut hasher);
        hasher.finish()
    };
    let name = format!("with_cache_bytes-{:x}", name_hash);

    let path = &projectdir().cache_dir().join(&name);

    if let Ok(f) = fs::File::open(path).await {
        if let Some(ttl) = ttl {
            if f.metadata().await?.modified()?.elapsed()? < *ttl {
                let r = f.read_to_end_at(Vec::new(), 0).await;
                let (_, buf) = buf_try!(@try r);
                log::trace!("cache hit: {}", name);
                return Ok(bytes::Bytes::from(buf));
            }
        }
    }

    let r = fut.await?;
    fs::create_dir_all(path.parent().unwrap()).await?;
    let (_, r) = buf_try!(@try fs::write(path, r).await);

    Ok(r)
}
