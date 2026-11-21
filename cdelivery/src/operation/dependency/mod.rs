pub(crate) mod typ;

use crate::usecase;

super::define!(
    PlanetScaleSizes, typ::PlanetScaleSizesQuery => typ::PlanetScaleSizes,
    "dependency_planetscale_sizes", "List PlanetScale network-storage rehearsal sizes, optionally matching a dependency's cloud provider and CPU architecture. Availability for an organization is determined by PlanetScale when creating the branch.", Shared,
    super::Effect::Read, Standard, super::Media::Json;
    |app, _access, input| {
        let dependency = match input.dependency {
            Some(name) => Some(usecase::dependency::get(&app.docker, &name.parse()?).await?),
            None => None,
        };
        Ok(usecase::dependency::planetscale_sizes(dependency.as_ref())?.into())
    }
);

super::define!(
    List, typ::Query => typ::Dependencies,
    "dependency_list", "List PostgreSQL dependency metadata. A group filter returns local dependencies in that group and all global cloud dependencies. Credentials and DSNs are never returned; apps are available through app_list.", Shared,
    super::Effect::Read, Standard, super::Media::Json;
    |app, _access, input| {
        let group = input.group.as_deref().map(str::parse).transpose()?;
        Ok(typ::Dependencies {
            dependencies: usecase::dependency::list(&app.docker, group.as_ref())
                .await?.into_iter().map(Into::into).collect(),
        })
    }
);

super::define!(
    Get, super::input::Name => typ::Dependency,
    "dependency_get", "Get immutable PostgreSQL dependency metadata without its credentials or DSN.", Shared,
    super::Effect::Read, Standard, super::Media::Json;
    |app, _access, input| {
        Ok(usecase::dependency::get(&app.docker, &input.name.parse()?).await?.into())
    }
);

super::define!(
    Create, typ::CreateRequest => super::Done,
    "dependency_create", "Create an immutable PostgreSQL dependency. Cloud databases are global. Local databases inherit the source app's group and can only be referenced in that group. Replacing configuration requires a new dependency.", Admin,
    super::Effect::Write { destructive: false, idempotent: false }, Standard, super::Media::Json;
    |app, _access, input| {
        usecase::dependency::create(&app.docker, &app.key, usecase::dependency::CreateParams {
            name: input.name.parse()?, description: input.description,
            dsn: input.dsn, provider: input.provider.into(), api_token: input.api_token,
            project_id: input.project_id, app: input.app.as_deref().map(str::parse).transpose()?,
        }).await?;
        Ok(super::Done::new())
    }
);

super::define!(
    Delete, super::input::Name => super::Done,
    "dependency_delete", "Delete an unused PostgreSQL dependency. App references and unfinished changes prevent deletion.", Admin,
    super::Effect::Write { destructive: true, idempotent: true }, Standard, super::Media::Json;
    |app, _access, input| {
        usecase::dependency::delete(&app.docker, &app.key, &input.name.parse()?).await?;
        Ok(super::Done::new())
    }
);
