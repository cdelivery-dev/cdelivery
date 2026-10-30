pub(crate) mod typ;

super::define!(
    Get, super::input::Name => typ::Guardian,
    "guardian_get", "Get an app's pending guarded update.", Shared,
    super::Effect::Read, Standard, super::Media::Json;
    |app, _access, input, now| {
        get(app, &input.name, now).await
    }
);

super::define!(
    Retry, super::input::Name => super::Done,
    "guardian_retry", "Retry the failed guarded update check.", Operator,
    super::Effect::Write { destructive: true, idempotent: false },
    Standard, super::Media::Json;
    |app, _access, input| {
        crate::usecase::guardian::retry(&app.docker, &app.key, &input.name.parse()?).await?;
        Ok(super::Done::new())
    }
);

super::define!(
    Skip, super::input::Name => super::Done,
    "guardian_skip", "Release a pending update without passing its health check.", Operator,
    super::Effect::Write { destructive: true, idempotent: true },
    Standard, super::Media::Json;
    |app, _access, input, now| {
        crate::usecase::guardian::skip(&app.docker, &app.key, &input.name.parse()?, now).await?;
        Ok(super::Done::new())
    }
);

super::define!(
    Cancel, super::input::Name => super::Done,
    "guardian_cancel", "Cancel a guarded update and discard its changes.", Operator,
    super::Effect::Write { destructive: true, idempotent: true },
    Standard, super::Media::Json;
    |app, _access, input, now| {
        crate::usecase::guardian::cancel(&app.docker, &app.key, &input.name.parse()?, now).await?;
        Ok(super::Done::new())
    }
);
pub(super) async fn get(
    app: &crate::boot::App,
    name: &str,
    now: jiff::Timestamp,
) -> crate::error::Result<typ::Guardian> {
    crate::usecase::guardian::get(&app.docker, &app.key, &name.parse()?, now)
        .await?
        .try_into()
}
