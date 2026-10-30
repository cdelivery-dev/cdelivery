use serde::Serialize;
use serde_json::{Map, Value};

use crate::concept;
use crate::error::{Error, ErrorKind, Result};

pub(crate) mod apikey;
pub(crate) mod app;
pub(crate) mod catalog;
pub(crate) mod event;
pub(crate) mod group;
pub(crate) mod guardian;
pub(crate) mod input;
pub(crate) mod insight;
pub(crate) mod instance;
pub(crate) mod node;
pub(crate) mod overview;
pub(crate) mod pingress;
pub(crate) mod schema;
pub(crate) mod settings;
pub(crate) mod typ;
pub(crate) mod user;
pub(crate) mod validate;
pub(crate) mod volume;
pub(crate) mod xhealth;
pub(crate) mod xjob;

macro_rules! define {
    (
        $name:ident, $input:ty => $output:ty,
        $id:literal, $description:literal, $requirement:ident, $effect:expr,
        $budget:ident, $media:expr;
        $(validate |$validate_input:ident, $validate_now:ident, $validate_report:ident| $validate_body:block;)?
        |$app:ident, $access:ident, $value:ident $(, $now:ident)?| $body:block
    ) => {
        pub(crate) struct $name;

        impl super::Operation for $name {
            type Input = $input;
            type Output = $output;

            const ID: &'static str = $id;
            const DESCRIPTION: &'static str = $description;
            const REQUIREMENT: crate::concept::permission::Requirement =
                crate::concept::permission::Requirement::$requirement;
            const EFFECT: super::Effect = $effect;
            const BUDGET: super::Budget = super::Budget::$budget;
            const MEDIA: super::Media = $media;

            $(
                fn validate_at(
                    input: &Self::Input,
                    now: jiff::Timestamp,
                    report: &mut garde::Report,
                ) {
                    let $validate_input = input;
                    let $validate_now = now;
                    let $validate_report = report;
                    $validate_body
                }
            )?

            fn run(
                app: &crate::boot::App,
                access: &crate::concept::permission::Access,
                value: Self::Input,
                _now: jiff::Timestamp,
            ) -> impl Future<Output = crate::error::Result<Self::Output>> + Send {
                let $app = app;
                let $access = access;
                let $value = value;
                $(let $now = _now;)?
                async move $body
            }
        }
    };
}

pub(super) use define;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Effect {
    Read,
    Write { destructive: bool, idempotent: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Budget {
    Standard,
    Extended,
}

impl Budget {
    pub(crate) const fn duration(self) -> jiff::SignedDuration {
        match self {
            Self::Standard => crate::config::OPERATION_STANDARD_TIMEOUT,
            Self::Extended => crate::config::OPERATION_EXTENDED_TIMEOUT,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Media {
    Json,
    Artifact(&'static str),
}

impl Media {
    pub(crate) fn accepts(self, payload: &Payload) -> bool {
        matches!(
            (self, payload),
            (Self::Json, Payload::Json(_)) | (Self::Artifact(_), Payload::Artifact(_))
        )
    }

    pub(crate) const fn matches(self, mime: Option<&str>) -> bool {
        match (self, mime) {
            (Self::Json, None) => true,
            (Self::Artifact(actual), Some(expected)) => same(actual, expected),
            _ => false,
        }
    }
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct Done {
    ok: bool,
}

impl Done {
    pub(crate) const fn new() -> Self {
        Self { ok: true }
    }
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct Artifact {
    pub(crate) uri: String,
    pub(crate) filename: String,
    pub(crate) bytes: Vec<u8>,
}

#[derive(Debug)]
pub(crate) enum Payload {
    Json(Map<String, Value>),
    Artifact(ArtifactPayload),
}

#[derive(Debug)]
pub(crate) struct ArtifactPayload {
    pub(crate) uri: String,
    pub(crate) filename: String,
    pub(crate) mime: &'static str,
    pub(crate) bytes: Vec<u8>,
}

pub(crate) fn json_payload(value: impl Serialize) -> Result<Payload> {
    match serde_json::to_value(value)? {
        Value::Object(object) => Ok(Payload::Json(object)),
        _ => Err(ErrorKind::InvalidOperationOutput.into()),
    }
}

pub(crate) fn artifact_payload<O: Operation<Output = Artifact>>(
    artifact: Artifact,
) -> Result<Payload> {
    let Media::Artifact(mime) = O::MEDIA else {
        return Err(ErrorKind::InvalidOperationOutput.into());
    };
    Ok(Payload::Artifact(ArtifactPayload {
        uri: artifact.uri,
        filename: artifact.filename,
        mime,
        bytes: artifact.bytes,
    }))
}

pub(crate) trait Operation {
    type Input: serde::de::DeserializeOwned
        + garde::Validate<Context = ()>
        + utoipa::ToSchema
        + Send
        + Sync;
    type Output: Serialize + utoipa::ToSchema + Send;

    const ID: &'static str;
    const DESCRIPTION: &'static str;
    const REQUIREMENT: concept::permission::Requirement;
    const EFFECT: Effect;
    const BUDGET: Budget;
    const MEDIA: Media;

    fn run(
        app: &crate::boot::App,
        access: &concept::permission::Access,
        input: Self::Input,
        now: jiff::Timestamp,
    ) -> impl Future<Output = Result<Self::Output>> + Send;

    fn validate_at(_input: &Self::Input, _now: jiff::Timestamp, _report: &mut garde::Report) {}

    fn payload(output: Self::Output) -> Result<Payload> {
        json_payload(output)
    }
}

pub(crate) async fn execute<O: Operation + catalog::Catalogued>(
    app: &crate::boot::App,
    access: &concept::permission::Access,
    input: O::Input,
    now: jiff::Timestamp,
) -> Result<O::Output> {
    validate_input::<O>(&input, now)?;
    if !access.allows(O::REQUIREMENT) {
        return Err(ErrorKind::Forbidden.into());
    }
    let actor = access.actor().clone();
    let output = crate::infra::event::ACTOR
        .scope(actor, async {
            tokio::time::timeout(
                O::BUDGET.duration().unsigned_abs(),
                O::run(app, access, input, now),
            )
            .await
            .map_err(|source| {
                Error::new(ErrorKind::OperationTimeout {
                    operation: O::ID,
                    source,
                })
            })?
        })
        .await?;
    validate_output::<O>(&output)?;
    Ok(output)
}

pub(crate) fn validate_input<O: Operation>(input: &O::Input, now: jiff::Timestamp) -> Result<()> {
    let mut report = validate::report(input);
    O::validate_at(input, now, &mut report);
    if !report.is_empty() {
        return Err(report.into());
    }
    Ok(())
}

fn validate_output<O: Operation>(output: &O::Output) -> Result<()> {
    if matches!(O::MEDIA, Media::Json) && !serde_json::to_value(output)?.is_object() {
        return Err(ErrorKind::InvalidOperationOutput.into());
    }
    Ok(())
}

#[expect(clippy::indexing_slicing)]
pub(crate) const fn same(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    if left.len() != right.len() {
        return false;
    }
    let mut index = 0;
    while index < left.len() {
        if left[index] != right[index] {
            return false;
        }
        index += 1;
    }
    true
}

pub(crate) fn problem_json(error: &Error) -> Value {
    serde_json::json!(error.problem())
}
