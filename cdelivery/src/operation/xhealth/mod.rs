pub(crate) mod typ;

super::input::composite_input!(CreateInput {
    #[garde(custom(super::validate::valid_name))]
    app: String,
    #[garde(dive)]
    spec: typ::CreateXHealthRequest,
});

super::input::composite_input!(UpdateInput {
    #[garde(custom(super::validate::valid_name))]
    app: String,
    #[garde(dive)]
    changes: typ::UpdateXHealthRequest,
});

super::define!(
    Get, super::input::App => typ::XHealth,
    "xhealth_get", "Get an app's health check configuration and current periodic check round. Pending, Running and Settled do not establish recovery; Settled means no result is available. Failed.error is a task diagnostic; use app_logs with target XHealth for check output. Guardian checks the candidate service separately; use guardian_get for the pending update's status.", Shared,
    super::Effect::Read, Standard, super::Media::Json;
    |app, _access, input, now| {
        crate::usecase::xhealth::get(&app.docker, &input.app.parse()?, now)
            .await?
            .try_into()
    }
);

super::define!(
    Create, CreateInput => typ::XHealth,
    "xhealth_create", "Enable app health checks. Hurl requires file and version. Custom commands run as argv in a separate container using the app image; localhost refers to that check container. TARGET_HOST and TARGET_PORT identify the app service for periodic checks and the candidate service for Guardian. Use an explicit shell to expand variables, e.g. command: [\"sh\",\"-c\",\"wget -O - \\\"http://${TARGET_HOST}:${TARGET_PORT}/\\\"\"]. This example requires sh and wget in the image. Reference {{TARGET_URL}} in Hurl files.", Operator,
    super::Effect::Write { destructive: false, idempotent: false },
    Standard, super::Media::Json;
    |app, _access, input, now| {
        crate::usecase::xhealth::create(
            &app.docker,
            &app.key,
            &app.releases,
            &input.app.parse()?,
            input.spec.method.into(),
            input.spec.interval.into(),
            input.spec.target.try_into()?,
            input.spec.guardian,
            now,
        )
        .await
        .map_err(|error| error.nested("spec"))?
        .try_into()
    }
);

super::define!(
    Update, UpdateInput => typ::XHealth,
    "xhealth_update", "Patch app health checks. Existing Hurl accepts file, version, or both; switching to Hurl requires both. Custom commands run as argv in a separate container using the app image; localhost refers to that check container. TARGET_HOST and TARGET_PORT identify the app service for periodic checks and the candidate service for Guardian. Use an explicit shell to expand variables, e.g. command: [\"sh\",\"-c\",\"wget -O - \\\"http://${TARGET_HOST}:${TARGET_PORT}/\\\"\"]. This example requires sh and wget in the image. Reference {{TARGET_URL}} in Hurl files.", Operator,
    super::Effect::Write { destructive: true, idempotent: false },
    Standard, super::Media::Json;
    |app, _access, input, now| {
        crate::usecase::xhealth::update(
            &app.docker,
            &app.key,
            &app.releases,
            &input.app.parse()?,
            input.changes.method.map(Into::into),
            input.changes.interval.map(Into::into),
            input.changes.target.map(TryInto::try_into).transpose()?,
            input.changes.guardian,
            now,
        )
        .await
        .map_err(|error| error.nested("changes"))?
        .try_into()
    }
);

super::define!(
    Run, super::input::App => super::Done,
    "xhealth_run", "Run an enabled health check now using the saved configuration against the current app. Rejects Pending or Running checks. Acceptance does not mean the check has started or passed; use xhealth_get and app_logs with target XHealth for the current status and output. The interval restarts from this round's start. Guardian candidate checks are separate. This operation is not idempotent: after a timeout or lost response, inspect the current state before deciding whether to run again; do not retry automatically.", Operator,
    super::Effect::Write { destructive: true, idempotent: false },
    Standard, super::Media::Json;
    |app, _access, input, now| {
        crate::usecase::xhealth::run_now(&app.docker, &app.key, &input.app.parse()?, now)
            .await?;
        Ok(super::Done::new())
    }
);

super::define!(
    Disable, super::input::App => super::Done,
    "xhealth_disable", "Disable app health checks.", Operator,
    super::Effect::Write { destructive: true, idempotent: true },
    Standard, super::Media::Json;
    |app, _access, input| {
        crate::usecase::xhealth::disable(&app.docker, &input.app.parse()?).await?;
        Ok(super::Done::new())
    }
);

pub(crate) struct DownloadFile;

impl super::Operation for DownloadFile {
    type Input = super::input::App;
    type Output = super::Artifact;

    const ID: &'static str = "xhealth_download_file";
    const DESCRIPTION: &'static str = "Download an app's Hurl health check file.";
    const REQUIREMENT: crate::concept::permission::Requirement =
        crate::concept::permission::Requirement::Shared;
    const EFFECT: super::Effect = super::Effect::Read;
    const BUDGET: super::Budget = super::Budget::Standard;
    const MEDIA: super::Media = super::Media::Artifact("text/plain");

    async fn run(
        app: &crate::boot::App,
        _access: &crate::concept::permission::Access,
        input: Self::Input,
        _now: jiff::Timestamp,
    ) -> crate::error::Result<Self::Output> {
        let bytes =
            crate::usecase::xhealth::download_file(&app.docker, &input.app.parse()?).await?;
        Ok(super::Artifact {
            uri: format!("cdelivery:/apps/{}/xhealth/file", input.app),
            filename: format!("{}.hurl", input.app),
            bytes,
        })
    }

    fn payload(output: Self::Output) -> crate::error::Result<super::Payload> {
        super::artifact_payload::<Self>(output)
    }
}
