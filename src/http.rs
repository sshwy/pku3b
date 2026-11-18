//! Small HTTP client helpers for `pku3b`.
//!
//! This module wraps `cyper` to provide a lightweight, cloneable client that can
//! persist cookies observed via `Set-Cookie` response headers.
use std::sync::{Arc, RwLock};

use cookie_store::CookieStore;
use cyper::{Body, IntoUrl, Response};
use http::{HeaderName, HeaderValue};
use serde::Serialize;

/// Format a URL for logs without user credentials, query parameters or fragments.
pub(crate) fn log_url(url: &str) -> String {
    let Ok(mut url) = url.into_url() else {
        return "[invalid URL]".to_owned();
    };
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    url.to_string()
}

#[derive(Debug, Clone)]
/// A `cyper` client with a shared, persistable cookie store.
///
/// Cookies are collected from `Set-Cookie` response headers when requests are
/// sent via [`RequestBuilder::send`]. The store can be saved to / loaded from
/// disk as JSON.
pub struct Client {
    http_client: cyper::Client,
    cookie_store: Arc<RwLock<CookieStore>>,
}

impl Client {
    /// Wrap an existing `cyper::Client` and start with an empty cookie store.
    pub fn from_cyper(client: cyper::Client) -> Self {
        Self {
            http_client: client,
            cookie_store: Arc::new(RwLock::default()),
        }
    }

    /// Start building a GET request.
    pub fn get<U: IntoUrl>(&self, url: U) -> cyper::Result<RequestBuilder> {
        Ok(RequestBuilder {
            builder: self.http_client.get(url)?,
            cookie_store: self.cookie_store.clone(),
        })
    }

    /// Start building a POST request.
    pub fn post<U: IntoUrl>(&self, url: U) -> cyper::Result<RequestBuilder> {
        Ok(RequestBuilder {
            builder: self.http_client.post(url)?,
            cookie_store: self.cookie_store.clone(),
        })
    }

    /// Save the current cookie store to a JSON file.
    pub async fn save_set_cookies<P: AsRef<std::path::Path>>(&self, path: P) -> anyhow::Result<()> {
        let mut buf = Vec::new();
        {
            let cookie_store = self.cookie_store.read().unwrap();
            cookie_store::serde::json::save_incl_expired_and_nonpersistent(&cookie_store, &mut buf)
                .map_err(|e| {
                    anyhow::anyhow!(
                        "save cookie store to {} failed: {e}",
                        path.as_ref().display()
                    )
                })?;
        }
        if let Some(parent) = path.as_ref().parent() {
            compio::fs::create_dir_all(parent).await?;
        }
        compio::fs::write(path.as_ref(), buf).await.0?;
        Ok(())
    }

    /// Load the cookie store from a JSON file, replacing any existing cookies.
    pub async fn load_set_cookies<P: AsRef<std::path::Path>>(&self, path: P) -> anyhow::Result<()> {
        let data = compio::fs::read(path.as_ref()).await?;
        let cookie_store: CookieStore =
            cookie_store::serde::json::load_all(&mut std::io::Cursor::new(data)).map_err(|e| {
                anyhow::anyhow!(
                    "load cookie store from {} failed: {e}",
                    path.as_ref().display()
                )
            })?;

        *self.cookie_store.write().unwrap() = cookie_store;
        Ok(())
    }
}

/// A wrapper around `cyper::RequestBuilder` that can record response cookies.
///
/// The underlying request is built using `cyper` APIs. When the request is sent
/// via [`send`](Self::send), any `Set-Cookie` response headers are appended to
/// the shared cookie store.
pub struct RequestBuilder {
    builder: cyper::RequestBuilder,
    cookie_store: Arc<RwLock<CookieStore>>,
}

impl RequestBuilder {
    /// Add URL query parameters.
    pub fn query<T: Serialize + ?Sized>(self, query: &T) -> cyper::Result<RequestBuilder> {
        Ok(RequestBuilder {
            builder: self.builder.query(query)?,
            cookie_store: self.cookie_store,
        })
    }

    /// Add a single HTTP header.
    pub fn header<K: TryInto<HeaderName>, V: TryInto<HeaderValue>>(
        self,
        key: K,
        value: V,
    ) -> cyper::Result<RequestBuilder>
    where
        K::Error: Into<http::Error>,
        V::Error: Into<http::Error>,
    {
        Ok(RequestBuilder {
            builder: self.builder.header(key, value)?,
            cookie_store: self.cookie_store,
        })
    }

    /// Set the request body.
    pub fn body<T: Into<Body>>(self, body: T) -> RequestBuilder {
        RequestBuilder {
            builder: self.builder.body(body),
            cookie_store: self.cookie_store,
        }
    }

    /// Set a `application/x-www-form-urlencoded` request body.
    pub fn form<T: Serialize + ?Sized>(self, form: &T) -> cyper::Result<RequestBuilder> {
        Ok(RequestBuilder {
            builder: self.builder.form(form)?,
            cookie_store: self.cookie_store,
        })
    }

    /// Send the request and record any `Set-Cookie` response headers.
    pub async fn send(self) -> cyper::Result<Response> {
        let (c, mut req) = self.builder.build_split();
        let url = req.url().clone();

        {
            let cookie_value = self
                .cookie_store
                .read()
                .unwrap()
                .get_request_values(&url)
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join("; ");

            if !cookie_value.is_empty()
                && let Ok(v) = HeaderValue::from_str(&cookie_value)
            {
                log::trace!("override cookies for {}", log_url(url.as_str()));
                req.headers_mut().insert(http::header::COOKIE, v);
            }
        }

        let res = c.execute(req).await?;
        for (k, v) in res.headers() {
            if k == http::header::SET_COOKIE
                && let Ok(v) = v.to_str()
            {
                let v = v.to_owned();
                if let Ok(c) = cookie_store::Cookie::parse(v, &url) {
                    log::trace!("record response cookie for {}", log_url(url.as_str()));
                    let _ = self.cookie_store.write().unwrap().insert(c, &url);
                }
            }
        }
        Ok(res)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_url_omits_credentials_query_and_fragment() {
        assert_eq!(
            log_url("https://user:password@example.com/sso?token=secret#secret"),
            "https://example.com/sso"
        );
        assert_eq!(log_url("not a URL?token=secret"), "[invalid URL]");
    }

    struct TestLogger;

    thread_local! {
        static MESSAGES: std::cell::RefCell<Vec<String>> = const {
            std::cell::RefCell::new(Vec::new())
        };
    }

    impl log::Log for TestLogger {
        fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
            metadata.target().starts_with("pku3b::")
        }

        fn log(&self, record: &log::Record<'_>) {
            if self.enabled(record.metadata()) {
                MESSAGES.with(|messages| messages.borrow_mut().push(record.args().to_string()));
            }
        }

        fn flush(&self) {}
    }

    #[compio::test]
    async fn cookie_round_trip_does_not_log_secrets() {
        use std::io::{Read, Write};
        static LOGGER: TestLogger = TestLogger;
        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(|| {
            log::set_logger(&LOGGER).unwrap();
            log::set_max_level(log::LevelFilter::Trace);
        });
        MESSAGES.with(|messages| messages.borrow_mut().clear());

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for i in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buf = [0; 1024];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = stream.read(&mut buf).unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&buf[..n]);
                }
                requests.push(String::from_utf8(request).unwrap());
                let cookie = if i == 0 {
                    "Set-Cookie: session=synthetic-cookie-secret; Path=/; HttpOnly\r\n"
                } else {
                    ""
                };
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n{cookie}\r\n"
                )
                .unwrap();
            }
            requests
        });

        let client = Client::from_cyper(cyper::Client::builder().no_proxy().build().unwrap());
        for _ in 0..2 {
            client
                .get(format!("http://{addr}/sso?token=synthetic-token-secret"))
                .unwrap()
                .send()
                .await
                .unwrap();
        }
        let requests = server.join().unwrap();
        assert!(requests[1].contains("session=synthetic-cookie-secret"));
        assert!(requests[1].contains("token=synthetic-token-secret"));
        MESSAGES.with(|messages| {
            let logs = messages.borrow().join("\n");
            assert!(logs.contains("record response cookie"));
            assert!(logs.contains("override cookies"));
            assert!(logs.contains("/sso"));
            assert!(!logs.contains("synthetic-cookie-secret"));
            assert!(!logs.contains("synthetic-token-secret"));
        });
    }

    fn test_cookie_path() -> (std::path::PathBuf, std::path::PathBuf) {
        let base = std::env::temp_dir().join(format!(
            "pku3b-http-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = base.join("nested").join("ua.json");
        (base, path)
    }

    #[compio::test]
    async fn save_set_cookies_creates_parent_dirs() {
        let client =
            Client::from_cyper(cyper::Client::builder().cookie_store(true).build().unwrap());
        let (base, path) = test_cookie_path();

        client.save_set_cookies(&path).await.unwrap();

        assert!(path.exists(), "cookie file should be created");

        std::fs::remove_dir_all(base).unwrap();
    }
}
