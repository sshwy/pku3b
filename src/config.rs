#[cfg(feature = "keyring")]
use anyhow::Context as _;
use compio::fs;
#[cfg(not(feature = "keyring"))]
use std::convert::Infallible;
#[cfg(feature = "keyring")]
use std::path::Path;

#[cfg(feature = "keyring")]
const KEYRING_SERVICE: &str = "org.sshwy.pku3b";

#[derive(serde::Deserialize, serde::Serialize, Clone, Copy, Default)]
#[serde(rename_all = "lowercase")]
pub enum SecretBackend {
    #[default]
    Plaintext,
    Keyring,
}

fn default_true() -> bool {
    true
}

#[derive(serde::Deserialize, serde::Serialize, Clone)]
pub struct Config {
    pub username: String,
    #[serde(default)]
    pub password: String,
    pub ttshitu: Option<TTShiTuConfig>,
    pub bark: Option<BarkConfig>,
    #[serde(default)]
    pub secret_backend: SecretBackend,

    pub auto_supplement: Option<Vec<SupplementCourseConfig>>,

    /// 默认的 TA 课程 ID（如 "_98207_1"）
    #[serde(default)]
    pub ta_course_id: Option<String>,
    /// 默认的批改组 ID
    #[serde(default)]
    pub ta_group_id: Option<String>,
    /// 下载提交文件时自动重命名为 {学生姓名}_{作业名}_{原始名}
    #[serde(default = "default_true")]
    pub ta_rename_files: bool,
    /// 每个学生只下载最新一次提交
    #[serde(default = "default_true")]
    pub ta_latest_only: bool,
}

#[derive(serde::Deserialize, serde::Serialize, Clone)]
pub struct SupplementCourseConfig {
    pub page_id: usize,
    pub name: String,
    pub teacher: String,
    pub class_id: String,
}

#[derive(serde::Deserialize, serde::Serialize, Clone)]
pub struct TTShiTuConfig {
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
}

#[derive(serde::Deserialize, serde::Serialize, Clone)]
pub struct BarkConfig {
    #[serde(default)]
    pub token: String,
}

impl Config {
    pub fn redacted(&self) -> Self {
        let mut cfg = self.clone();
        cfg.redact_for_storage();
        cfg
    }

    pub fn display(&self, attr: ConfigAttrs, buf: &mut Vec<u8>) -> anyhow::Result<()> {
        use std::io::Write as _;
        match attr {
            ConfigAttrs::Username => writeln!(buf, "{}", self.username)?,
            ConfigAttrs::Password => writeln!(buf, "{}", self.password)?,
            ConfigAttrs::TTShiTuUsername => {
                if let Some(tt) = &self.ttshitu {
                    writeln!(buf, "{}", tt.username)?
                } else {
                    writeln!(buf, "<not set>")?
                }
            }
            ConfigAttrs::TTShiTuPassword => {
                if let Some(tt) = &self.ttshitu {
                    writeln!(buf, "{}", tt.password)?
                } else {
                    writeln!(buf, "<not set>")?
                }
            }
            ConfigAttrs::BarkToken => {
                if let Some(bark) = &self.bark {
                    writeln!(buf, "{}", bark.token)?
                } else {
                    writeln!(buf, "<not set>")?
                }
            }
            ConfigAttrs::SecretBackend => {
                let backend = match self.secret_backend {
                    SecretBackend::Plaintext => "plaintext",
                    SecretBackend::Keyring => "keyring",
                };
                writeln!(buf, "{backend}")?
            }
            ConfigAttrs::TaCourseId => {
                if let Some(cid) = &self.ta_course_id {
                    writeln!(buf, "{cid}")?
                } else {
                    writeln!(buf, "<not set>")?
                }
            }
            ConfigAttrs::TaGroupId => {
                if let Some(gid) = &self.ta_group_id {
                    writeln!(buf, "{gid}")?
                } else {
                    writeln!(buf, "<not set>")?
                }
            }
            ConfigAttrs::TaRenameFiles => writeln!(buf, "{}", self.ta_rename_files)?,
            ConfigAttrs::TaLatestOnly => writeln!(buf, "{}", self.ta_latest_only)?,
        };
        Ok(())
    }

    pub fn update(&mut self, attr: ConfigAttrs, value: String) -> anyhow::Result<()> {
        match attr {
            ConfigAttrs::Username => self.username = value,
            ConfigAttrs::Password => self.password = value,
            ConfigAttrs::TTShiTuUsername => {
                if let Some(tt) = &mut self.ttshitu {
                    tt.username = value
                } else {
                    self.ttshitu = Some(TTShiTuConfig {
                        username: value,
                        password: String::new(),
                    })
                }
            }
            ConfigAttrs::TTShiTuPassword => {
                if let Some(tt) = &mut self.ttshitu {
                    tt.password = value
                } else {
                    self.ttshitu = Some(TTShiTuConfig {
                        username: String::new(),
                        password: value,
                    })
                }
            }
            ConfigAttrs::BarkToken => self.bark = Some(BarkConfig { token: value }),
            ConfigAttrs::SecretBackend => {
                self.secret_backend = match value.to_ascii_lowercase().as_str() {
                    "plaintext" => SecretBackend::Plaintext,
                    "keyring" => SecretBackend::Keyring,
                    _ => anyhow::bail!(
                        "invalid secret backend: {value} (expected: plaintext | keyring)"
                    ),
                };
            }
            ConfigAttrs::TaCourseId => self.ta_course_id = Some(value),
            ConfigAttrs::TaGroupId => self.ta_group_id = Some(value),
            ConfigAttrs::TaRenameFiles => {
                self.ta_rename_files = value
                    .parse()
                    .map_err(|_| anyhow::anyhow!("expected true or false"))?
            }
            ConfigAttrs::TaLatestOnly => {
                self.ta_latest_only = value
                    .parse()
                    .map_err(|_| anyhow::anyhow!("expected true or false"))?
            }
        }

        Ok(())
    }

    #[cfg(feature = "keyring")]
    fn keyring_account(path: &Path, key: &str) -> String {
        let path = path
            .canonicalize()
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .into_owned();
        format!("config:{path}:{key}")
    }

    #[cfg(feature = "keyring")]
    fn keyring_set(path: &Path, key: &str, value: &str) -> anyhow::Result<()> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, &Self::keyring_account(path, key))?;
        entry.set_password(value)?;
        Ok(())
    }

    #[cfg(feature = "keyring")]
    fn keyring_get(path: &Path, key: &str) -> anyhow::Result<Option<String>> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, &Self::keyring_account(path, key))?;
        match entry.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    #[cfg(feature = "keyring")]
    fn keyring_get_required(path: &Path, key: &str) -> anyhow::Result<String> {
        Self::keyring_get(path, key)?
            .with_context(|| format!("secret `{key}` not found in keyring"))
    }

    #[cfg(feature = "keyring")]
    fn keyring_delete(path: &Path, key: &str) -> anyhow::Result<()> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, &Self::keyring_account(path, key))?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    #[cfg(feature = "keyring")]
    fn sync_to_keyring(&self, path: &Path) -> anyhow::Result<()> {
        Self::keyring_set(path, "password", &self.password)?;
        if let Some(tt) = &self.ttshitu {
            Self::keyring_set(path, "ttshitu.username", &tt.username)?;
            Self::keyring_set(path, "ttshitu.password", &tt.password)?;
        }
        if let Some(bark) = &self.bark {
            Self::keyring_set(path, "bark.token", &bark.token)?;
        }
        Ok(())
    }

    #[cfg(feature = "keyring")]
    fn fill_from_keyring(&mut self, path: &Path) -> anyhow::Result<()> {
        self.password = Self::keyring_get_required(path, "password")?;
        if let Some(tt) = &mut self.ttshitu {
            tt.username = Self::keyring_get_required(path, "ttshitu.username")?;
            tt.password = Self::keyring_get_required(path, "ttshitu.password")?;
        }
        if let Some(bark) = &mut self.bark {
            bark.token = Self::keyring_get_required(path, "bark.token")?;
        }
        Ok(())
    }

    #[cfg(feature = "keyring")]
    fn remove_from_keyring(path: &Path) {
        for key in [
            "password",
            "ttshitu.username",
            "ttshitu.password",
            "bark.token",
        ] {
            if let Err(e) = Self::keyring_delete(path, key) {
                log::warn!("failed to remove keyring secret `{key}`: {e:#}");
            }
        }
    }

    #[cfg(not(feature = "keyring"))]
    fn ensure_keyring_enabled() -> anyhow::Result<Infallible> {
        anyhow::bail!(
            "secret_backend is set to keyring, but this binary was built without the `keyring` feature"
        )
    }

    fn redact_for_storage(&mut self) {
        self.password.clear();
        if let Some(tt) = &mut self.ttshitu {
            tt.username.clear();
            tt.password.clear();
        }
        if let Some(bark) = &mut self.bark {
            bark.token.clear();
        }
    }
}

#[derive(Debug, Clone)]
pub enum ConfigAttrs {
    Username,
    Password,
    TTShiTuUsername,
    TTShiTuPassword,
    BarkToken,
    SecretBackend,
    TaCourseId,
    TaGroupId,
    TaRenameFiles,
    TaLatestOnly,
}

impl clap::ValueEnum for ConfigAttrs {
    fn value_variants<'a>() -> &'a [Self] {
        &[
            Self::Username,
            Self::Password,
            Self::TTShiTuUsername,
            Self::TTShiTuPassword,
            Self::BarkToken,
            Self::SecretBackend,
            Self::TaCourseId,
            Self::TaGroupId,
            Self::TaRenameFiles,
            Self::TaLatestOnly,
        ]
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        match self {
            Self::Username => Some(clap::builder::PossibleValue::new("username")),
            Self::Password => Some(clap::builder::PossibleValue::new("password")),
            Self::TTShiTuUsername => Some(clap::builder::PossibleValue::new("ttshitu.username")),
            Self::TTShiTuPassword => Some(clap::builder::PossibleValue::new("ttshitu.password")),
            Self::BarkToken => Some(clap::builder::PossibleValue::new("bark.token")),
            Self::SecretBackend => Some(clap::builder::PossibleValue::new("secret-backend")),
            Self::TaCourseId => Some(clap::builder::PossibleValue::new("ta-course-id")),
            Self::TaGroupId => Some(clap::builder::PossibleValue::new("ta-group-id")),
            Self::TaRenameFiles => Some(clap::builder::PossibleValue::new("ta-rename-files")),
            Self::TaLatestOnly => Some(clap::builder::PossibleValue::new("ta-latest-only")),
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
        anyhow::bail!("file not found");
    }

    let buffer = fs::read(path).await?;
    let content = String::from_utf8(buffer)?; //.context("invalid UTF-8")?;
    let cfg: Config = toml::from_str(&content)?;
    if matches!(cfg.secret_backend, SecretBackend::Keyring) {
        #[cfg(feature = "keyring")]
        {
            let mut cfg = cfg;
            cfg.fill_from_keyring(path)?;
            return Ok(cfg);
        }
        #[cfg(not(feature = "keyring"))]
        Config::ensure_keyring_enabled()?;
    }

    Ok(cfg)
}

pub async fn write_cfg(path: impl AsRef<std::path::Path>, cfg: &Config) -> anyhow::Result<()> {
    let path = path.as_ref();
    // Create the parent directory if it does not exist
    if let Some(par) = path.parent()
        && !par.exists()
    {
        fs::create_dir_all(par).await?;
    }

    let cfg_to_write = if matches!(cfg.secret_backend, SecretBackend::Keyring) {
        #[cfg(feature = "keyring")]
        {
            let mut cfg_to_write = cfg.clone();
            cfg_to_write.sync_to_keyring(path)?;
            cfg_to_write.redact_for_storage();
            cfg_to_write
        }
        #[cfg(not(feature = "keyring"))]
        {
            match Config::ensure_keyring_enabled()? {}
        }
    } else {
        #[cfg(feature = "keyring")]
        {
            Config::remove_from_keyring(path);
        }
        cfg.clone()
    };

    let content = toml::to_string(&cfg_to_write)?;
    fs::write(path, content).await.0?;
    Ok(())
}
