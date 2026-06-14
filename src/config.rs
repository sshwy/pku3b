use compio::fs;
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
        if let Some(v) = Self::keyring_get(path, "password")? {
            self.password = v;
        }
        if let Some(tt) = &mut self.ttshitu {
            if let Some(v) = Self::keyring_get(path, "ttshitu.username")? {
                tt.username = v;
            }
            if let Some(v) = Self::keyring_get(path, "ttshitu.password")? {
                tt.password = v;
            }
        }
        if let Some(bark) = &mut self.bark
            && let Some(v) = Self::keyring_get(path, "bark.token")?
        {
            bark.token = v;
        }
        Ok(())
    }

    #[cfg(feature = "keyring")]
    fn remove_from_keyring(path: &Path) -> anyhow::Result<()> {
        for key in [
            "password",
            "ttshitu.username",
            "ttshitu.password",
            "bark.token",
        ] {
            Self::keyring_delete(path, key)?;
        }
        Ok(())
    }

    #[cfg(not(feature = "keyring"))]
    fn ensure_keyring_enabled() -> anyhow::Result<()> {
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
            Config::ensure_keyring_enabled()?;
            unreachable!("ensure_keyring_enabled always returns an error")
        }
    } else {
        #[cfg(feature = "keyring")]
        {
            Config::remove_from_keyring(path)?;
        }
        cfg.clone()
    };

    let content = toml::to_string(&cfg_to_write)?;
    fs::write(path, content).await.0?;
    Ok(())
}
