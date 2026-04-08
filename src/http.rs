//! Small HTTP client helpers for `pku3b`.
//!
//! This module wraps `cyper` to provide a lightweight, cloneable client that can
//! persist cookies observed via `Set-Cookie` response headers.
use std::sync::{Arc, RwLock};

use cyper::{Body, IntoUrl, Response};
use http::{HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
struct SetCookie {
    /// The effective domain that produced the `Set-Cookie` header.
    domain: String,
    /// The request path associated with the response that set this cookie.
    path: String,
    /// The raw `Set-Cookie` header value.
    value: String,
}

type SetCookies = Vec<SetCookie>;

#[derive(Debug, Clone)]
/// A `cyper` client with a shared, persistable cookie store.
///
/// Cookies are collected from `Set-Cookie` response headers when requests are
/// sent via [`RequestBuilder::send`]. The store can be saved to / loaded from
/// disk as JSON.
pub struct Client {
    http_client: cyper::Client,
    set_cookies: Arc<RwLock<SetCookies>>,
}

impl Client {
    /// Wrap an existing `cyper::Client` and start with an empty cookie store.
    pub fn from_cyper(client: cyper::Client) -> Self {
        Self {
            http_client: client,
            set_cookies: Arc::new(RwLock::default()),
        }
    }

    /// Start building a GET request.
    pub fn get<U: IntoUrl>(&self, url: U) -> cyper::Result<RequestBuilder> {
        Ok(RequestBuilder {
            builder: self.http_client.get(url)?,
            set_cookies: self.set_cookies.clone(),
        })
    }

    /// Start building a POST request.
    pub fn post<U: IntoUrl>(&self, url: U) -> cyper::Result<RequestBuilder> {
        Ok(RequestBuilder {
            builder: self.http_client.post(url)?,
            set_cookies: self.set_cookies.clone(),
        })
    }

    /// Save the current cookie store to a JSON file.
    pub async fn save_set_cookies<P: AsRef<std::path::Path>>(&self, path: P) -> anyhow::Result<()> {
        let set_cookies = self.set_cookies.read().unwrap();
        let data = serde_json::to_string(&*set_cookies)?;
        compio::buf::buf_try!(@try compio::fs::write(path, data).await);
        Ok(())
    }

    /// Load the cookie store from a JSON file, replacing any existing cookies.
    pub async fn load_set_cookies<P: AsRef<std::path::Path>>(&self, path: P) -> anyhow::Result<()> {
        let data = compio::fs::read(path).await?;
        let set_cookies: SetCookies = serde_json::from_slice(&data)?;

        *self.set_cookies.write().unwrap() = set_cookies;
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
    set_cookies: Arc<RwLock<Vec<SetCookie>>>,
}

impl RequestBuilder {
    /// Add URL query parameters.
    pub fn query<T: Serialize + ?Sized>(self, query: &T) -> cyper::Result<RequestBuilder> {
        Ok(RequestBuilder {
            builder: self.builder.query(query)?,
            set_cookies: self.set_cookies,
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
            set_cookies: self.set_cookies,
        })
    }

    /// Set the request body.
    pub fn body<T: Into<Body>>(self, body: T) -> RequestBuilder {
        RequestBuilder {
            builder: self.builder.body(body),
            set_cookies: self.set_cookies,
        }
    }

    /// Set a `application/x-www-form-urlencoded` request body.
    pub fn form<T: Serialize + ?Sized>(self, form: &T) -> cyper::Result<RequestBuilder> {
        Ok(RequestBuilder {
            builder: self.builder.form(form)?,
            set_cookies: self.set_cookies,
        })
    }

    /// Send the request and record any `Set-Cookie` response headers.
    pub async fn send(self) -> cyper::Result<Response> {
        let (c, req) = self.builder.build_split();
        let domain = req.url().domain().map(|s| s.to_string());
        let path = req.url().path().to_string();
        let res = c.execute(req).await?;
        if let Some(domain) = domain {
            for (k, v) in res.headers() {
                if k == HeaderName::from_static("set-cookie")
                    && let Ok(v) = v.to_str()
                {
                    log::debug!("set cookie for {domain}{path}: {}", v);
                    self.set_cookies.write().unwrap().push(SetCookie {
                        domain: domain.clone(),
                        path: path.clone(),
                        value: v.to_string(),
                    });
                }
            }
        }
        Ok(res)
    }
}
