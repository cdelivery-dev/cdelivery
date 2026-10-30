pub(crate) mod typ;

super::define!(
    Create, typ::CreateApiKeyRequest => typ::IssuedApiKey,
    "apikey_create", "Create an API key at or below your effective role; the key is returned only once.", Own,
    super::Effect::Write { destructive: false, idempotent: false },
    Standard, super::Media::Json;
    validate |input, now, report| {
        super::validate::append(
            report,
            garde::Path::new("expires_at"),
            typ::future_or_absent(input.expires_at, now),
        );
    };
    |app, access, input, now| {
        let role = input.role.into();
        if access.role() < role {
            return Err(crate::error::ErrorKind::Forbidden.into());
        }
        let (key, record) = crate::usecase::user::issue_api_key(
            &app.docker,
            &app.key,
            access.user_id()?,
            input.name,
            input.expires_at,
            role,
            now,
        )
        .await?;
        let typ::ApiKey {
            id,
            name,
            created_at,
            expires_at,
            role,
        } = record.into();
        Ok(typ::IssuedApiKey {
            id,
            name,
            key,
            created_at,
            expires_at,
            role,
        })
    }
);

super::define!(
    List, super::input::Empty => typ::ApiKeys,
    "apikey_list", "List the current user's API keys.", Own,
    super::Effect::Read, Standard, super::Media::Json;
    |app, access, _input| {
        Ok(typ::ApiKeys {
            keys: crate::usecase::user::list_api_keys(&app.docker, &app.key, access.user_id()?)
                .await?
                .into_iter()
                .map(typ::ApiKey::from)
                .collect(),
        })
    }
);

super::define!(
    Revoke, super::input::ApiKeyId => super::Done,
    "apikey_revoke", "Revoke an API key.", Own,
    super::Effect::Write { destructive: true, idempotent: true },
    Standard, super::Media::Json;
    |app, access, input| {
        crate::usecase::user::revoke_api_key(
            &app.docker,
            &app.key,
            access.user_id()?,
            input.id,
        )
        .await?;
        Ok(super::Done::new())
    }
);
