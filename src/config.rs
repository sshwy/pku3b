use compio::fs;

#[derive(serde::Deserialize, serde::Serialize)]
pub struct Config {
    pub username: String,
    pub password: String,
}

impl Config {
    pub fn display(
        &self,
        attr: ConfigAttrs,
        buf: &mut Vec<u8>,
    ) -> anyhow::Result<()> {
        use std::io::Write as _;
        Ok(match attr {
            ConfigAttrs::Username => writeln!(buf, "{}", self.username)?,
            ConfigAttrs::Password => writeln!(buf, "{}", self.password)?,
        })
    }

    pub fn update(&mut self, attr: ConfigAttrs, value: String) -> anyhow::Result<()> {
        match attr {
            ConfigAttrs::Username => self.username = value,
            ConfigAttrs::Password => self.password = value,
        }

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum ConfigAttrs {
    Username,
    Password,
}

impl clap::ValueEnum for ConfigAttrs {
    fn value_variants<'a>() -> &'a [Self] {
        &[Self::Username, Self::Password]
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        match self {
            Self::Username => Some(clap::builder::PossibleValue::new("username")),
            Self::Password => Some(clap::builder::PossibleValue::new("password")),
        }
    }
}

/// Reads the configuration from the specified file path asynchronously.
///
/// # Errors
///
/// This function will return an error if:
/// - The file does not exist.
/// - The file cannot be opened.
/// - The file contents cannot be read.
/// - The file contents cannot be parsed as TOML.
///
pub async fn read_cfg(path: impl AsRef<std::path::Path>) -> anyhow::Result<Config> {
    let path = path.as_ref();

    if !path.exists() {
        anyhow::bail!("config file not found");
    }

    let buffer = fs::read(path).await?;
    let content = String::from_utf8(buffer)?; //.context("invalid UTF-8")?;
    let cfg: Config = toml::from_str(&content)?;

    Ok(cfg)
}

pub async fn write_cfg(path: impl AsRef<std::path::Path>, cfg: &Config) -> anyhow::Result<()> {
    let path = path.as_ref();
    // Create the parent directory if it does not exist
    if let Some(par) = path.parent() {
        if !par.exists() {
            fs::create_dir_all(par).await?;
        }
    }

    let content = toml::to_string(cfg)?;
    fs::write(path, content).await.0?;
    Ok(())
}
