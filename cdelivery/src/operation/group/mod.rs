pub(crate) mod typ;

use crate::usecase;

super::input::composite_input!(UpdateInput {
    #[garde(custom(super::validate::valid_name))]
    name: String,
    #[garde(dive)]
    changes: typ::UpdateGroupRequest,
});

super::define!(
    List, super::input::Empty => typ::Groups,
    "group_list", "List groups.", Shared, super::Effect::Read,
    Standard, super::Media::Json;
    |app, _access, _input| {
        Ok(typ::Groups {
            groups: usecase::group::list(&app.docker)
                .await?
                .into_iter()
                .map(Into::into)
                .collect(),
        })
    }
);

super::define!(
    Get, super::input::Name => typ::Group,
    "group_get", "Get a group by name.", Shared, super::Effect::Read,
    Standard, super::Media::Json;
    |app, _access, input| {
        Ok(usecase::group::get(&app.docker, &input.name.parse()?)
            .await?
            .into())
    }
);

super::define!(
    Create, typ::CreateGroupRequest => typ::Group,
    "group_create", "Create a group.", Admin,
    super::Effect::Write { destructive: false, idempotent: false },
    Standard, super::Media::Json;
    |app, _access, input| {
        Ok(
            usecase::group::create(&app.docker, input.name.parse()?, input.description)
                .await?
                .into(),
        )
    }
);

super::define!(
    Update, UpdateInput => typ::Group,
    "group_update", "Patch a group.", Admin,
    super::Effect::Write { destructive: true, idempotent: true },
    Standard, super::Media::Json;
    |app, _access, input| {
        Ok(
            usecase::group::update(&app.docker, &input.name.parse()?, input.changes.description)
                .await?
                .into(),
        )
    }
);

super::define!(
    Delete, super::input::Name => super::Done,
    "group_delete", "Delete a group.", Admin,
    super::Effect::Write { destructive: true, idempotent: true },
    Standard, super::Media::Json;
    |app, _access, input| {
        usecase::group::delete(&app.docker, &input.name.parse()?).await?;
        Ok(super::Done::new())
    }
);
