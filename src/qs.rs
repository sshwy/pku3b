//! Simple query string parser.

use std::str::FromStr;

pub struct Query {
    qs: Vec<String>,
}

impl Query {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.qs
            .iter()
            .find(|&s| s.starts_with(key))
            .map(|s| s.split_at(key.len() + 1).1)
    }
}

impl FromStr for Query {
    type Err = <http::Uri as FromStr>::Err;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let uri = http::Uri::from_str(s)?;
        let qs = uri
            .query()
            .map(|q| q.split('&').map(ToOwned::to_owned).collect::<Vec<_>>())
            .unwrap_or_default();
        Ok(Self { qs })
    }
}
