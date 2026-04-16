//! Small HTTP client helpers for `pku3b`.
//!
//! This module wraps `cyper` to provide a lightweight, cloneable client that can
//! persist cookies observed via `Set-Cookie` response headers.
use std::sync::{Arc, RwLock};

use cookie_store::CookieStore;
use cyper::{Body, IntoUrl, Response};
use http::{HeaderName, HeaderValue};
use serde::Serialize;

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
                log::trace!("override cookie for {url}: {cookie_value}");
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
                    log::trace!("set cookie for {url}: {:?}", c);
                    let _ = self.cookie_store.write().unwrap().insert(c, &url);
                }
            }
        }
        Ok(res)
    }
}
