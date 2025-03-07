pub mod style {
    use clap::builder::styling::{AnsiColor, Color, Style};

    pub const D: Style = Style::new().dimmed();
    pub const H1: Style = Style::new().bold().underline();
    pub const H2: Style = Style::new().underline();
    pub const H3: Style = Style::new().italic();
    pub const GR: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::Green)));
    pub const MG: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::BrightMagenta)));
    pub const BL: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::Cyan)));
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

    let path = &projectdir().cache_dir().join(name);
    // dbg!(path.display());
    if let Ok(f) = std::fs::File::open(path) {
        if let Some(ttl) = ttl {
            if f.metadata()?.modified()?.elapsed()? < *ttl {
                if let Ok(r) = serde_json::from_reader(f) {
                    return Ok(r);
                }
            }
        }
    }

    let r = fut.await?;
    std::fs::create_dir_all(path.parent().unwrap())?;
    let f = std::fs::File::create(path)?;
    serde_json::to_writer(f, &r)?;

    Ok(r)
}
