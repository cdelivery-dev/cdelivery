use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const PORT: u16 = 5012;
pub const KEY_PATH: &str = "/run/secrets/cdelivery-postgres";
pub const SECRET_DIRECTORY: &str = "/run/cdelivery/postgres";
pub const MAX_SECRET_BYTES: usize = 511_999;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanetScaleSku {
    pub name: String,
    pub size: String,
    pub memory_bytes: u64,
    pub provider: String,
    pub architecture: String,
    pub metal: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Command {
    Prepare {
        dependency: String,
        name: String,
        host: Option<String>,
        size: Option<String>,
        planetscale_sizes: Option<Vec<PlanetScaleSku>>,
    },
    Execute {
        dependency: String,
        target: Target,
        sql: cipher::Secret,
    },
    Cleanup {
        dependency: String,
        name: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Target {
    Preview,
    Production,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Response {
    Prepared { dsn: cipher::Secret },
    Done,
    Failed { message: String },
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ErrorKind {
    #[error("invalid PostgreSQL URI: {0}")]
    Uri(#[source] url::ParseError),
    #[error("invalid PostgreSQL URI: {0}")]
    InvalidDsn(&'static str),
    #[error("secret fields must be UTF-8: {0}")]
    Utf8(#[source] std::str::Utf8Error),
    #[error("invalid flat secret field encoding")]
    Fields,
    #[error("secret exceeds 511999 bytes")]
    TooLarge,
}

macros::located_error! {
    Error => ErrorKind,
    from {
        url::ParseError => Uri,
        std::str::Utf8Error => Utf8,
    }
}

pub struct Dsn {
    raw: String,
    host: String,
    port: u16,
    database: String,
    username: String,
    password: String,
}

impl std::fmt::Debug for Dsn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dsn").finish_non_exhaustive()
    }
}

impl Dsn {
    pub fn parse(raw: &str) -> Result<Self> {
        if raw.contains('\0') || raw.trim() != raw {
            return Err(
                ErrorKind::InvalidDsn("NUL and surrounding whitespace are forbidden").into(),
            );
        }
        let uri = url::Url::parse(raw)?;
        if !matches!(uri.scheme(), "postgres" | "postgresql") || uri.fragment().is_some() {
            return Err(ErrorKind::InvalidDsn("use a PostgreSQL URI without a fragment").into());
        }
        let host = uri
            .host()
            .ok_or(ErrorKind::InvalidDsn("host is required"))?
            .to_string();
        let host = host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_owned();
        if host.is_empty() || host.contains([',', '%', '/']) {
            return Err(ErrorKind::InvalidDsn("one TCP host is required").into());
        }
        let decode = |value: &str| -> Result<String> {
            let decoded = percent_encoding::percent_decode_str(value).decode_utf8()?;
            if decoded.contains('\0') {
                return Err(ErrorKind::InvalidDsn("decoded fields cannot contain NUL").into());
            }
            Ok(decoded.into_owned())
        };
        let username = decode(uri.username())?;
        let database = decode(uri.path().strip_prefix('/').unwrap_or(uri.path()))?;
        if username.is_empty()
            || database.is_empty()
            || uri.path().trim_start_matches('/').contains('/')
        {
            return Err(ErrorKind::InvalidDsn("username and database are required").into());
        }
        let port = uri.port().unwrap_or(5432);
        if port == 0 {
            return Err(ErrorKind::InvalidDsn("port must be between 1 and 65535").into());
        }
        Ok(Self {
            raw: raw.to_owned(),
            host,
            port,
            database,
            username,
            password: decode(uri.password().unwrap_or_default())?,
        })
    }

    pub fn dsn(&self) -> &str {
        &self.raw
    }
    pub fn host(&self) -> &str {
        &self.host
    }
    pub fn port(&self) -> u16 {
        self.port
    }
    pub fn database(&self) -> &str {
        &self.database
    }
    pub fn username(&self) -> &str {
        &self.username
    }
    pub fn password(&self) -> &str {
        &self.password
    }

    pub fn fields(&self) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("dsn".to_owned(), self.raw.clone()),
            ("host".to_owned(), self.host.clone()),
            ("port".to_owned(), self.port.to_string()),
            ("database".to_owned(), self.database.clone()),
            ("username".to_owned(), self.username.clone()),
            ("password".to_owned(), self.password.clone()),
        ])
    }
}

pub fn rewrite_dsn(original: &str, target: Option<&str>, host: Option<&str>) -> Result<String> {
    Dsn::parse(original)?;
    let mut uri = url::Url::parse(target.unwrap_or(original))?;
    Dsn::parse(uri.as_str())?;
    if let Some(host) = host {
        uri.set_host(Some(host))?;
    }
    uri.set_query(None);
    let mut output = uri.to_string();
    if let Some((_, query)) = original.split_once('?') {
        output.push('?');
        output.push_str(query);
    }
    Dsn::parse(&output)?;
    Ok(output)
}

pub fn encode_fields(fields: &BTreeMap<String, String>) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    for (key, value) in fields {
        if key.is_empty()
            || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            || value.contains('\0')
        {
            return Err(ErrorKind::Fields.into());
        }
        output.extend_from_slice(key.as_bytes());
        output.push(b'=');
        output.extend_from_slice(value.as_bytes());
        output.push(0);
    }
    if output.len() > MAX_SECRET_BYTES {
        return Err(ErrorKind::TooLarge.into());
    }
    Ok(output)
}

pub fn parse_fields(bytes: &[u8]) -> Result<BTreeMap<String, String>> {
    if bytes.len() > MAX_SECRET_BYTES {
        return Err(ErrorKind::TooLarge.into());
    }
    if !bytes.ends_with(&[0]) {
        return Err(ErrorKind::Fields.into());
    }
    let mut fields = BTreeMap::new();
    for entry in std::str::from_utf8(bytes)?.split_terminator('\0') {
        let (key, value) = entry.split_once('=').ok_or(ErrorKind::Fields)?;
        if key.is_empty()
            || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            || fields.insert(key.to_owned(), value.to_owned()).is_some()
        {
            return Err(ErrorKind::Fields.into());
        }
    }
    Ok(fields)
}
