//! Simple query string parser.

use std::str::FromStr;

pub struct Query {
    qs: Vec<String>,
}

impl Query {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.qs
            .iter()
            .filter_map(|s| s.split_once('='))
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_whole_key() {
        let q = Query::from_str("https://example.invalid/p?sub_ids=1,2&sub_id=3&app_id=4").unwrap();
        assert_eq!(q.get("sub_id"), Some("3"));
        assert_eq!(q.get("app_id"), Some("4"));
        assert_eq!(q.get("course_id"), None);
    }

    #[test]
    fn handles_parameter_without_value() {
        let q = Query::from_str("https://example.invalid/p?auth_data&auth_data_x=1").unwrap();
        assert_eq!(q.get("auth_data"), None);
    }
}
