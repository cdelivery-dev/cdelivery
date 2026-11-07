pub(crate) mod typ;

use crate::usecase;

super::define!(
    List, typ::Query => typ::Secrets,
    "secret_list", "List user Secret metadata, optionally filtered by type. Content is never returned.", Shared,
    super::Effect::Read, Standard, super::Media::Json;
    |app, _access, input| {
        Ok(typ::Secrets { secrets: usecase::secret::list(&app.docker, input.kind.map(Into::into))
            .await?.into_iter().map(Into::into).collect() })
    }
);

super::define!(
    Create, typ::CreateRequest => super::Done,
    "secret_create", "Create an immutable Secret. Text is only for environment variables; File is only for XConfig mounts. Content cannot be read back.", Operator,
    super::Effect::Write { destructive: false, idempotent: false }, Standard, super::Media::Json;
    |app, _access, input| {
        let (kind, data) = input.content.decode()?;
        usecase::secret::create(&app.docker, input.name.parse()?, kind, input.description, data).await?;
        Ok(super::Done::new())
    }
);

super::define!(
    Delete, super::input::Name => super::Done,
    "secret_delete", "Delete an unused user Secret. Current service references and available app rollback configurations prevent deletion.", Operator,
    super::Effect::Write { destructive: true, idempotent: true }, Standard, super::Media::Json;
    |app, _access, input| {
        usecase::secret::delete(&app.docker, &input.name.parse()?).await?;
        Ok(super::Done::new())
    }
);
