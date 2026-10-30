use garde::Validate;
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::concept;

pub(crate) mod log;
pub(crate) mod metric;

pub(crate) const fn default_query_limit() -> u16 {
    100
}

pub(crate) fn write_only<T>(_: &T) -> bool {
    true
}

macro_rules! mirror_struct {
    ($($target:ident)::+ from $value:ident: $source:ty {
        $($field:ident: $from:expr),+ $(,)?
    }) => {
        impl From<$source> for $($target)::+ {
            fn from($value: $source) -> Self {
                Self { $($field: $from),+ }
            }
        }
    };
    ($wire:ident($w:ident: $wt:ty) <-> $($c:ident)::+ ($v:ident: $ct:ty) {
        $($f:ident: $to:expr => $from:expr),+ $(,)?
    }) => {
        impl From<$wt> for $($c)::+ {
            fn from($w: $wt) -> Self {
                Self::new($($to),+)
            }
        }

        impl From<$ct> for $wire {
            fn from($v: $ct) -> Self {
                Self { $($f: $from),+ }
            }
        }
    };
    ($wire:ident($w:ident: $wt:ty) <-> $($c:ident)::+ ($v:ident: $ct:ty) try {
        $($f:ident: $to:expr => $from:expr),+ $(,)?
    }) => {
        impl From<$wt> for $($c)::+ {
            fn from($w: $wt) -> Self {
                Self::new($($to),+)
            }
        }

        impl TryFrom<$ct> for $wire {
            type Error = $crate::error::Error;

            fn try_from($v: $ct) -> $crate::error::Result<Self> {
                Ok(Self { $($f: $from),+ })
            }
        }
    };
}

pub(crate) use mirror_struct;

macros::mirror_enum! {
    #[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
    Role both concept::user::Role {
        Reader,
        Operator,
        Admin,
    }
}

macros::mirror_enum! {
    #[derive(
        Debug,
        Clone,
        Copy,
        PartialEq,
        Eq,
        PartialOrd,
        Ord,
        Serialize,
        Deserialize,
        ToSchema,
        strum::VariantArray,
    )]
    Im both concept::message::Im {
        Feishu,
        Lark,
        Slack,
        Telegram,
        Discord,
    }
}

macros::mirror_enum! {
    #[derive(Debug, Serialize, ToSchema)]
    #[schema(as = CheckStatus)]
    RunState from concept::run::State {
        Pending,
        Running { started_at: Timestamp },
        Passed { finished_at: Timestamp },
        Failed { error: String, finished_at: Timestamp },
        Settled,
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Validate)]
#[serde(tag = "unit", content = "value", deny_unknown_fields)]
pub(crate) enum Duration {
    Seconds(#[garde(range(min = 1, max = 1_000_000))] u64),
    Minutes(#[garde(range(min = 1, max = 1_000_000))] u64),
    Hours(#[garde(range(min = 1, max = 1_000_000))] u64),
}

impl utoipa::PartialSchema for Duration {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        quantity_schema(["Seconds", "Minutes", "Hours"], 1_000_000)
    }
}

impl ToSchema for Duration {}

impl From<Duration> for SignedDuration {
    #[expect(clippy::cast_possible_wrap)]
    fn from(duration: Duration) -> Self {
        match duration {
            Duration::Seconds(value) => Self::from_secs(value as i64),
            Duration::Minutes(value) => Self::from_mins(value as i64),
            Duration::Hours(value) => Self::from_hours(value as i64),
        }
    }
}

impl TryFrom<SignedDuration> for Duration {
    type Error = crate::error::Error;

    fn try_from(duration: SignedDuration) -> crate::error::Result<Self> {
        let nanos = duration.as_nanos();
        let second = 1_000_000_000;
        if nanos <= 0 || nanos % second != 0 {
            return Err(crate::error::ErrorKind::InvalidOperationOutput.into());
        }
        let seconds = u64::try_from(nanos / second)
            .map_err(|_| crate::error::ErrorKind::InvalidOperationOutput)?;
        let duration = if seconds.is_multiple_of(3600) && seconds / 3600 <= 1_000_000 {
            Self::Hours(seconds / 3600)
        } else if seconds.is_multiple_of(60) && seconds / 60 <= 1_000_000 {
            Self::Minutes(seconds / 60)
        } else if seconds <= 1_000_000 {
            Self::Seconds(seconds)
        } else {
            return Err(crate::error::ErrorKind::InvalidOperationOutput.into());
        };
        Ok(duration)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Validate)]
#[serde(tag = "unit", content = "value", deny_unknown_fields)]
pub(crate) enum ByteSize {
    MiB(#[garde(range(min = 1, max = 1_048_576))] u64),
    GiB(#[garde(range(min = 1, max = 1_048_576))] u64),
    TiB(#[garde(range(min = 1, max = 1_048_576))] u64),
}

impl utoipa::PartialSchema for ByteSize {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        quantity_schema(["MiB", "GiB", "TiB"], 1_048_576)
    }
}

impl ToSchema for ByteSize {}

fn quantity_schema<const N: usize>(
    units: [&str; N],
    maximum: u64,
) -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
    use utoipa::openapi::schema::{AdditionalProperties, Type};
    utoipa::openapi::ObjectBuilder::new()
        .schema_type(Type::Object)
        .additional_properties(Some(AdditionalProperties::FreeForm(false)))
        .property(
            "unit",
            utoipa::openapi::ObjectBuilder::new()
                .schema_type(Type::String)
                .enum_values(Some(units)),
        )
        .required("unit")
        .property(
            "value",
            utoipa::openapi::ObjectBuilder::new()
                .schema_type(Type::Integer)
                .minimum(Some(1))
                .maximum(Some(maximum)),
        )
        .required("value")
        .into()
}

impl From<ByteSize> for u64 {
    fn from(size: ByteSize) -> Self {
        let (value, unit) = match size {
            ByteSize::MiB(value) => (value, concept::app::MIB),
            ByteSize::GiB(value) => (value, 1 << 30),
            ByteSize::TiB(value) => (value, 1 << 40),
        };
        value.saturating_mul(unit)
    }
}

impl TryFrom<u64> for ByteSize {
    type Error = crate::error::Error;

    fn try_from(bytes: u64) -> crate::error::Result<Self> {
        let size = if bytes != 0 && bytes.is_multiple_of(1 << 40) && bytes >> 40 <= 1_048_576 {
            Self::TiB(bytes >> 40)
        } else if bytes != 0 && bytes.is_multiple_of(1 << 30) && bytes >> 30 <= 1_048_576 {
            Self::GiB(bytes >> 30)
        } else if bytes != 0
            && bytes.is_multiple_of(concept::app::MIB)
            && bytes / concept::app::MIB <= 1_048_576
        {
            Self::MiB(bytes / concept::app::MIB)
        } else {
            return Err(crate::error::ErrorKind::InvalidOperationOutput.into());
        };
        Ok(size)
    }
}
