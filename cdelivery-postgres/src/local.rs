use std::collections::BTreeMap;

use cdelivery_wire::postgres;

use crate::error::{ErrorKind, Result};

pub(crate) fn prepare(
    fields: &BTreeMap<String, String>,
    host: Option<&str>,
) -> Result<cipher::Secret> {
    let host =
        host.ok_or_else(|| ErrorKind::Invalid("local preview host is required".to_owned()))?;
    let dsn = postgres::rewrite_dsn(super::field(fields, "dsn")?, None, Some(host))?;
    Ok(cipher::Secret::new(dsn))
}
