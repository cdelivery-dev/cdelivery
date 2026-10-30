pub(crate) mod typ;

super::input::composite_input!(CreateInput {
    #[garde(custom(super::validate::valid_name))]
    app: String,
    #[garde(custom(super::validate::valid_name))]
    job: String,
    #[garde(dive)]
    spec: typ::CreateXJobRequest,
});

super::input::composite_input!(UpdateInput {
    #[garde(custom(super::validate::valid_name))]
    app: String,
    #[garde(custom(super::validate::valid_name))]
    job: String,
    #[garde(dive)]
    changes: typ::UpdateXJobRequest,
});

super::define!(
    List, super::input::App => typ::XJobs,
    "xjob_list", "List an app's scheduled jobs.", Shared, super::Effect::Read,
    Standard, super::Media::Json;
    |app, _access, input, now| {
        Ok(typ::XJobs {
            jobs: crate::usecase::xjob::list(&app.docker, &input.app.parse()?, now)
                .await?
                .into_iter()
                .map(TryInto::try_into)
                .collect::<crate::error::Result<_>>()?,
        })
    }
);

super::define!(
    Get, super::input::AppJob => typ::XJob,
    "xjob_get", "Get an app's scheduled job.", Shared, super::Effect::Read,
    Standard, super::Media::Json;
    |app, _access, input, now| {
        crate::usecase::xjob::get(&app.docker, &input.app.parse()?, &input.job.parse()?, now)
            .await?
            .try_into()
    }
);

super::define!(
    Create, CreateInput => typ::XJob,
    "xjob_create", "Create a scheduled job for an app.", Operator,
    super::Effect::Write { destructive: false, idempotent: false },
    Standard, super::Media::Json;
    |app, _access, input, now| {
        crate::usecase::xjob::create(
            &app.docker,
            &app.key,
            &input.app.parse()?,
            &input.job.parse()?,
            crate::concept::xjob::Spec::new(
                input.spec.interval.into(),
                input.spec.timeout.into(),
                input.spec.command,
            ),
            now,
        )
        .await?
        .try_into()
    }
);

super::define!(
    Update, UpdateInput => typ::XJob,
    "xjob_update", "Patch a scheduled job.", Operator,
    super::Effect::Write { destructive: true, idempotent: true },
    Standard, super::Media::Json;
    |app, _access, input, now| {
        crate::usecase::xjob::update(
            &app.docker,
            &app.key,
            &input.app.parse()?,
            &input.job.parse()?,
            input.changes.interval.map(Into::into),
            input.changes.timeout.map(Into::into),
            input.changes.command,
            now,
        )
        .await?
        .try_into()
    }
);

super::define!(
    Delete, super::input::AppJob => super::Done,
    "xjob_delete", "Delete a scheduled job.", Operator,
    super::Effect::Write { destructive: true, idempotent: true },
    Standard, super::Media::Json;
    |app, _access, input| {
        crate::usecase::xjob::delete(&app.docker, &input.app.parse()?, &input.job.parse()?)
            .await?;
        Ok(super::Done::new())
    }
);

super::define!(
    Run, super::input::AppJob => typ::XJob,
    "xjob_run", "Run a scheduled job now.", Operator,
    super::Effect::Write { destructive: true, idempotent: false },
    Standard, super::Media::Json;
    |app, _access, input, now| {
        crate::usecase::xjob::run_now(
            &app.docker,
            &app.key,
            &input.app.parse()?,
            &input.job.parse()?,
            now,
        )
        .await?
        .try_into()
    }
);
