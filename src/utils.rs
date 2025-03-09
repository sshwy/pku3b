use compio::{buf::buf_try, fs, io, io::AsyncReadAtExt};
use futures_util::lock::Mutex;
use std::sync::OnceLock;

pub mod style {
    use clap::builder::styling::{AnsiColor, Color, Style};

    pub const D: Style = Style::new().dimmed();
    pub const B: Style = Style::new().bold();
    pub const H1: Style = Style::new().bold().underline();
    pub const H2: Style = Style::new().underline();
    pub const H3: Style = EM;
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

/// A simple wrapper around [`fs::Stdin`] which use async-aware mutex and async buf reader, in order to provide async read_line functionality.
pub struct Stdin {
    inner: &'static Mutex<io::BufReader<fs::Stdin>>,
}

impl Stdin {
    pub async fn read_line(&self, buf: &mut String) -> std::io::Result<usize> {
        let mut vec_buf = Vec::new();
        let mut inner = self.inner.lock().await;
        read_until(&mut *inner, b'\n', &mut vec_buf).await?;
        *buf = String::from_utf8(vec_buf).unwrap();
        Ok(buf.len())
    }
}

pub fn stdin() -> Stdin {
    static INSTANCE: OnceLock<Mutex<io::BufReader<fs::Stdin>>> = OnceLock::new();
    Stdin {
        inner: INSTANCE.get_or_init(|| Mutex::new(io::BufReader::new(fs::stdin()))),
    }
}

async fn read_until<R: io::AsyncBufRead + ?Sized>(
    r: &mut R,
    delim: u8,
    buf: &mut Vec<u8>,
) -> std::io::Result<usize> {
    let mut read = 0;
    loop {
        let (done, used) = {
            let available = match r.fill_buf().await {
                Ok(n) => n,
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            match memchr::memchr(delim, available) {
                Some(i) => {
                    buf.extend_from_slice(&available[..=i]);
                    (true, i + 1)
                }
                None => {
                    buf.extend_from_slice(available);
                    (false, available.len())
                }
            }
        };
        r.consume(used);
        read += used;
        if done || used == 0 {
            return Ok(read);
        }
    }
}
