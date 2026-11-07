pub(crate) mod typ;

use crate::usecase;

super::input::composite_input!(ListInput {
    #[garde(custom(super::validate::valid_node_id))]
    id: String,
    #[garde(dive)]
    query: typ::Query,
});

super::input::composite_input!(CreateInput {
    #[garde(custom(super::validate::valid_node_id))]
    id: String,
    #[garde(dive)]
    request: typ::CreateVolumeRequest,
});

super::input::composite_input!(UpdateInput {
    #[garde(custom(super::validate::valid_node_id))]
    id: String,
    #[garde(custom(super::validate::valid_name))]
    name: String,
    #[garde(dive)]
    changes: typ::UpdateVolumeRequest,
});

super::define!(
    Get, super::input::Empty => typ::Settings,
    "volume_get", "Get the Volume agent image; null means disabled.", Shared,
    super::Effect::Read, Standard, super::Media::Json;
    |app, _access, _input| {
        Ok(typ::Settings {
            image: usecase::volume::get(&app.docker).await?,
        })
    }
);

super::define!(
    Enable, typ::Changes => super::Done,
    "volume_enable", "Deploy or update the Volume agent; version is required for creation and omission preserves an existing image.", Admin,
    super::Effect::Write { destructive: true, idempotent: true },
    Standard, super::Media::Json;
    |app, _access, input| {
        usecase::volume::enable(&app.docker, &app.key, &app.releases, input.version.as_deref()).await?;
        Ok(super::Done::new())
    }
);

super::define!(
    Disable, super::input::Empty => super::Done,
    "volume_disable", "Remove the Volume agent service, keeping all volumes and their data.", Admin,
    super::Effect::Write { destructive: true, idempotent: true },
    Standard, super::Media::Json;
    |app, _access, _input| {
        usecase::volume::disable(&app.docker).await?;
        Ok(super::Done::new())
    }
);

super::define!(
    List, ListInput => typ::Report,
    "node_volume_list", "List node volumes and disk capacity; sizes requests a synchronous size scan. Requires the Volume agent from volume_enable.", Shared,
    super::Effect::Read, Standard, super::Media::Json;
    |app, _access, input| {
        let (report, mut descriptions) =
            usecase::volume::list(&app.docker, &app.key, input.id, input.query.sizes).await?;
        Ok(typ::Report {
            volumes: report.volumes.into_iter().map(|(name, size_bytes)| typ::Volume {
                description: descriptions.remove(&name),
                name,
                size_bytes,
            }).collect(),
            total_bytes: report.total_bytes,
            available_bytes: report.available_bytes,
        })
    }
);

super::define!(
    Create, CreateInput => super::Done,
    "node_volume_create", "Create a local volume with an optional description; existing and system-reserved names are rejected. Requires the Volume agent from volume_enable.", Admin,
    super::Effect::Write { destructive: false, idempotent: false },
    Standard, super::Media::Json;
    |app, _access, input| {
        usecase::volume::create(
            &app.docker, &app.key, input.id, input.request.name, input.request.description,
        ).await?;
        Ok(super::Done::new())
    }
);

super::define!(
    Update, UpdateInput => super::Done,
    "node_volume_update", "Patch a volume description; omission preserves it and null clears it. Requires the Volume agent from volume_enable.", Admin,
    super::Effect::Write { destructive: true, idempotent: true },
    Standard, super::Media::Json;
    |app, _access, input| {
        usecase::volume::update(
            &app.docker, &app.key, input.id, input.name, input.changes.description,
        ).await?;
        Ok(super::Done::new())
    }
);

super::define!(
    Delete, typ::Target => super::Done,
    "node_volume_delete", "Remove a volume without force; Docker rejects volumes in use. Requires the Volume agent from volume_enable.", Admin,
    super::Effect::Write { destructive: true, idempotent: true },
    Standard, super::Media::Json;
    |app, _access, input| {
        usecase::volume::delete(&app.docker, &app.key, input.id, input.name).await?;
        Ok(super::Done::new())
    }
);
