pub(crate) mod typ;

use crate::{concept, usecase};

super::input::composite_input!(Id {
    #[garde(skip)]
    id: uuid::Uuid,
});

super::define!(
    List, super::input::Empty => typ::Changes,
    "change_list", "List retained release changes. Each target keeps its latest change; open changes retain participating app reservations across groups until cleaned up and completed or closed.", Shared,
    super::Effect::Read, Standard, super::Media::Json;
    |app, _access, _input| {
        Ok(typ::Changes {
            changes: usecase::change::list(&app.docker, &app.key)
                .await?.into_iter().map(TryInto::try_into).collect::<crate::error::Result<_>>()?,
        })
    }
);

super::define!(
    Get, Id => typ::Change,
    "change_get", "Get a release change, its fixed participating apps, available actions, stage, and diagnostics.", Shared,
    super::Effect::Read, Standard, super::Media::Json;
    |app, _access, input| {
        usecase::change::get(&app.docker, &app.key, input.id).await?.try_into()
    }
);

super::define!(
    Create, typ::CreateRequest => typ::Change,
    "change_create", "Submit PostgreSQL SQL as one release change. SQL runs unchanged, without an added transaction; failed or uncertain production execution is never replayed automatically.", Operator,
    super::Effect::Write { destructive: true, idempotent: false }, Standard, super::Media::Json;
    |app, access, input, now| {
        usecase::change::create_sql(
            &app.docker, &app.key, &app.releases, input.dependency.parse()?, cipher::Secret::new(input.sql),
            input.size, access.actor().operator(), now,
        ).await?.try_into()
    }
);

super::define!(
    Close, Id => super::Done,
    "change_close", "Cancel and clean up an idle open change before Finish. Does not roll back production changes; verify the production result first in Apply stage.", UserOperator,
    super::Effect::Write { destructive: true, idempotent: false }, Standard, super::Media::Json;
    |app, _access, input| {
        usecase::change::act(&app.docker, &app.key, &app.releases, input.id, concept::change::Action::Close).await?;
        Ok(super::Done::new())
    }
);

super::define!(
    Apply, Id => super::Done,
    "change_apply", "Clean up and apply an idle open change without rehearsal. In Apply stage this explicitly re-executes the production change, including the original SQL; verify previous production results first.", UserOperator,
    super::Effect::Write { destructive: true, idempotent: false }, Standard, super::Media::Json;
    |app, _access, input| {
        usecase::change::act(&app.docker, &app.key, &app.releases, input.id, concept::change::Action::Apply).await?;
        Ok(super::Done::new())
    }
);

super::define!(
    Complete, Id => super::Done,
    "change_complete", "Confirm the production result, clean up, and complete an idle open change in Apply or Finish stage. Does not execute the production change.", UserOperator,
    super::Effect::Write { destructive: true, idempotent: false }, Standard, super::Media::Json;
    |app, _access, input| {
        usecase::change::act(&app.docker, &app.key, &app.releases, input.id, concept::change::Action::Complete).await?;
        Ok(super::Done::new())
    }
);
