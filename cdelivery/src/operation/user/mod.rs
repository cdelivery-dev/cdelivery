pub(crate) mod typ;

async fn get(
    app: &crate::boot::App,
    user_id: uuid::Uuid,
) -> crate::error::Result<crate::concept::user::User> {
    crate::usecase::user::get(&app.docker, &app.key, user_id).await
}

super::input::composite_input!(UpdateUserInput {
    #[garde(skip)]
    id: uuid::Uuid,
    #[garde(dive)]
    changes: typ::UpdateUserRequest,
});

super::define!(
    List, super::input::Empty => typ::Users,
    "user_list", "List platform users.", Admin, super::Effect::Read,
    Standard, super::Media::Json;
    |app, _access, _input| {
        Ok(typ::Users {
            users: crate::usecase::user::list(&app.docker, &app.key)
                .await?
                .into_iter()
                .map(typ::ManagedUser::from)
                .collect(),
        })
    }
);

super::define!(
    Get, super::input::UserId => typ::ManagedUser,
    "user_get", "Get a user by ID.", Admin, super::Effect::Read,
    Standard, super::Media::Json;
    |app, _access, input| {
        Ok(
            crate::usecase::user::get_managed(&app.docker, &app.key, input.id)
                .await?
                .into(),
        )
    }
);

super::define!(
    Create, typ::CreateUserRequest => typ::ManagedUser,
    "user_create", "Create a platform user.", Admin,
    super::Effect::Write { destructive: false, idempotent: false },
    Standard, super::Media::Json;
    |app, _access, input| {
        Ok(crate::usecase::user::create_managed(
            &app.docker,
            &app.key,
            input.first_name,
            input.last_name,
            input.email,
            input.password_sha256,
            input.role.into(),
        )
        .await?
        .into())
    }
);

super::define!(
    Update, UpdateUserInput => typ::ManagedUser,
    "user_update", "Patch a user; omitted fields stay unchanged. Password reset signs out all sessions; API keys stay valid. TOTP reset is independent.", Admin,
    super::Effect::Write { destructive: true, idempotent: true },
    Standard, super::Media::Json;
    |app, access, input| {
        if !matches!(input.changes.role, None | Some(super::typ::Role::Admin))
            && access.actor().user_id() == Some(input.id)
        {
            return Err(crate::error::ErrorKind::SelfImmutable.into());
        }
        Ok(crate::usecase::user::update(
            &app.docker,
            &app.key,
            input.id,
            input.changes.first_name,
            input.changes.last_name,
            input.changes.email,
            input.changes.role.map(Into::into),
            input.changes.password_sha256,
            input.changes.reset_totp,
        )
        .await?
        .into())
    }
);

super::define!(
    Delete, super::input::UserId => super::Done,
    "user_delete", "Delete a platform user.", Admin,
    super::Effect::Write { destructive: true, idempotent: true },
    Standard, super::Media::Json;
    |app, access, input| {
        if access.actor().user_id() == Some(input.id) {
            return Err(crate::error::ErrorKind::SelfImmutable.into());
        }
        crate::usecase::user::delete(&app.docker, input.id).await?;
        Ok(super::Done::new())
    }
);

super::define!(
    MeGet, super::input::Empty => typ::SelfUser,
    "me_get", "Get the current user's profile.", Own, super::Effect::Read,
    Standard, super::Media::Json;
    |app, access, _input| {
        Ok(get(app, access.user_id()?).await?.into())
    }
);

super::define!(
    MeUpdate, typ::UpdateProfileRequest => super::Done,
    "me_update", "Patch the current user's profile.", Own,
    super::Effect::Write { destructive: true, idempotent: true },
    Standard, super::Media::Json;
    |app, access, input| {
        crate::usecase::user::update_profile(
            &app.docker,
            &app.key,
            access.user_id()?,
            input.first_name,
            input.last_name,
        )
        .await?;
        Ok(super::Done::new())
    }
);

super::define!(
    MeChangePassword, typ::ChangePasswordRequest => super::Done,
    "me_change_password", "Change your password and sign out all sessions; API keys stay valid.", Own,
    super::Effect::Write { destructive: true, idempotent: true },
    Standard, super::Media::Json;
    |app, access, input| {
        crate::usecase::user::change_password(
            &app.docker,
            &app.key,
            access.user_id()?,
            input.current_password_sha256,
            input.password_sha256,
        )
        .await?;
        Ok(super::Done::new())
    }
);

super::define!(
    MePrepareTotp, super::input::Empty => typ::TotpEnrollment,
    "me_prepare_totp", "Generate your TOTP enrollment secret.", Own,
    super::Effect::Read, Standard, super::Media::Json;
    |app, access, _input| {
        let (secret_base32, uri) = crate::usecase::totp::prepare(
            &app.docker,
            &app.key,
            access.user_id()?,
        )
        .await?;
        Ok(typ::TotpEnrollment { secret_base32, uri })
    }
);

super::define!(
    MeEnableTotp, typ::EnableTotpRequest => super::Done,
    "me_enable_totp", "Enable TOTP with your password and first code.", Own,
    super::Effect::Write { destructive: false, idempotent: false },
    Standard, super::Media::Json;
    |app, access, input, now| {
        let secret = totp_rs::Secret::try_from_base32(input.secret_base32)
            .map_err(|_| crate::error::ErrorKind::TotpCodeInvalid)?;
        crate::usecase::totp::enable(
            &app.docker,
            &app.key,
            access.user_id()?,
            input.current_password_sha256,
            secret,
            input.code,
            now,
        )
        .await?;
        Ok(super::Done::new())
    }
);

super::define!(
    MeDisableTotp, typ::DisableTotpRequest => super::Done,
    "me_disable_totp", "Disable TOTP with your password and current code.", Own,
    super::Effect::Write { destructive: true, idempotent: true },
    Standard, super::Media::Json;
    |app, access, input, now| {
        crate::usecase::totp::disable(
            &app.docker,
            &app.key,
            access.user_id()?,
            input.current_password_sha256,
            input.code,
            now,
        )
        .await?;
        Ok(super::Done::new())
    }
);

super::input::composite_input!(ImPlatformInput {
    #[garde(skip)]
    platform: super::typ::Im,
});

super::define!(
    MeGetImProfile, ImPlatformInput => typ::ImProfile,
    "me_get_im_profile", "Get the name and avatar for your bound IM account. Unbound accounts fail; unavailable profile fields are null.", Own,
    super::Effect::Read, Standard, super::Media::Json;
    |app, access, input| {
        let (name, avatar) = crate::usecase::user::observe_im_profile(
            &app.docker,
            &app.key,
            access.user_id()?,
            input.platform.into(),
        )
        .await?;
        Ok(typ::ImProfile { name, avatar })
    }
);

super::define!(
    MeGetIm, super::input::Empty => typ::ImBindings,
    "me_get_im", "Get your IM bindings.", Own,
    super::Effect::Read, Standard, super::Media::Json;
    |app, access, _input| {
        Ok(typ::ImBindings {
            bindings: get(app, access.user_id()?)
                .await?
                .im()
                .iter()
                .map(|(im, id)| ((*im).into(), id.clone()))
                .collect(),
        })
    }
);

super::define!(
    MeSetIm, typ::SetImRequest => super::Done,
    "me_set_im", "Bind or unbind your IM account.", Own,
    super::Effect::Write { destructive: true, idempotent: true },
    Standard, super::Media::Json;
    |app, access, input| {
        crate::usecase::user::update_im(
            &app.docker,
            &app.key,
            access.user_id()?,
            input.platform.into(),
            input.id,
        )
        .await?;
        Ok(super::Done::new())
    }
);

super::define!(
    MeUpdatePreferences, typ::UpdatePreferencesRequest => super::Done,
    "me_update_preferences", "Patch your UI and notification preferences.", Own,
    super::Effect::Write { destructive: true, idempotent: true },
    Standard, super::Media::Json;
    |app, access, input| {
        crate::usecase::user::update_preferences(
            &app.docker,
            &app.key,
            access.user_id()?,
            input.language,
            input.theme,
            input.alert_email,
            input
                .alerts
                .map(|alerts| alerts.into_iter().map(Into::into).collect()),
        )
        .await?;
        Ok(super::Done::new())
    }
);
