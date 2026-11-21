use serde_with::DeserializeFromStr;

use crate::operation;

#[derive(Clone, Copy, Debug, strum::EnumString, strum::IntoStaticStr, DeserializeFromStr)]
#[repr(i64)]
pub(super) enum Range {
    #[strum(serialize = "15m")]
    M15 = 15 * 60,
    #[strum(serialize = "1h")]
    H1 = 60 * 60,
    #[strum(serialize = "6h")]
    H6 = 6 * 60 * 60,
    #[strum(serialize = "24h")]
    H24 = 24 * 60 * 60,
    #[strum(serialize = "7d")]
    D7 = 7 * 24 * 60 * 60,
    #[strum(serialize = "31d")]
    D31 = 31 * 24 * 60 * 60,
}

impl Range {
    fn duration(self) -> jiff::SignedDuration {
        jiff::SignedDuration::from_secs(self as i64)
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PageQuery {
    range: Option<Range>,
    start: Option<jiff::Timestamp>,
    end: Option<jiff::Timestamp>,
}

pub(super) struct Window {
    pub(super) range: (jiff::Timestamp, jiff::Timestamp),
    pub(super) preset: Option<&'static str>,
}

impl Window {
    pub(super) fn page(uri: &http::Uri, now: jiff::Timestamp) -> crate::error::Result<Self> {
        let axum::extract::Query(query) = axum::extract::Query::<PageQuery>::try_from_uri(uri)
            .map_err(|error| crate::error::ErrorKind::InvalidBody(error.body_text()))?;
        Ok(Self::preset(query.range, query.start, query.end, now))
    }

    pub(super) fn validate(&self, now: jiff::Timestamp) -> crate::error::Result<()> {
        let mut report = garde::Report::new();
        operation::validate::append(
            &mut report,
            garde::Path::empty(),
            operation::validate::query_time_range(Some(self.range.0), Some(self.range.1), now),
        );
        if report.is_empty() {
            Ok(())
        } else {
            Err(crate::error::ErrorKind::Validation(report).into())
        }
    }

    pub(super) fn preset(
        preset: Option<Range>,
        start: Option<jiff::Timestamp>,
        end: Option<jiff::Timestamp>,
        now: jiff::Timestamp,
    ) -> Self {
        let selected = preset.or_else(|| (start.is_none() && end.is_none()).then_some(Range::H1));
        let range = match preset {
            Some(preset) => (
                now.saturating_sub(preset.duration())
                    .unwrap_or(jiff::Timestamp::MIN),
                now,
            ),
            None => operation::validate::query_range(start, end, now),
        };
        Self {
            range,
            preset: selected.map(Into::into),
        }
    }
}
