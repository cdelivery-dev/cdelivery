use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::{Arc, LazyLock};

use cipher::MasterKey;
use insight_wire::event::Kind;
use jiff::{SignedDuration, Timestamp};
use uuid::Uuid;

use crate::error::{ErrorKind, Result};
use crate::locale::Locale;
use crate::{concept, config, infra};

static ADMISSION: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static BUSY: LazyLock<parking_lot::Mutex<HashSet<Uuid>>> =
    LazyLock::new(|| parking_lot::Mutex::new(HashSet::new()));

struct Guard(Uuid);

impl Drop for Guard {
    fn drop(&mut self) {
        BUSY.lock().remove(&self.0);
    }
}

fn acquire(id: Uuid) -> Result<Guard> {
    if !BUSY.lock().insert(id) {
        return Err(ErrorKind::ChangeBusy(id.to_string()).into());
    }
    Ok(Guard(id))
}

pub(crate) fn busy(id: Uuid) -> bool {
    BUSY.lock().contains(&id)
}

pub(crate) async fn admission() -> tokio::sync::MutexGuard<'static, ()> {
    ADMISSION.lock().await
}

pub(crate) async fn get(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    id: Uuid,
) -> Result<concept::change::Change> {
    Ok(infra::change::load(docker, key, id)
        .await?
        .ok_or(ErrorKind::ChangeNotFound)?
        .into_inner())
}

pub(crate) async fn list(
    docker: &infra::docker::Docker,
    key: &MasterKey,
) -> Result<Vec<concept::change::Change>> {
    infra::change::list(docker, key, false).await
}

async fn update(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    id: Uuid,
    mutate: impl FnOnce(&mut concept::change::Change),
) -> Result<concept::change::Change> {
    let mut current = infra::change::load(docker, key, id)
        .await?
        .ok_or(ErrorKind::ChangeNotFound)?;
    if !current.get().active() {
        return Err(ErrorKind::ChangeActionInvalid.into());
    }
    mutate(current.get_mut());
    infra::change::save(docker, key, &current).await?;
    Ok(current.into_inner())
}

pub(crate) async fn ensure_app_available(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    app: &concept::app::App,
) -> Result<()> {
    for change in infra::change::list(docker, key, true).await? {
        if change.occupies(app.name()) {
            return Err(ErrorKind::ChangeBusy(change.id().to_string()).into());
        }
    }
    Ok(())
}

pub(crate) async fn ensure_preview_available(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    preview: &str,
) -> Result<()> {
    for change in infra::change::list(docker, key, true).await? {
        if change
            .apps()
            .iter()
            .any(|app| infra::app::preview::clone_name(change.id(), app.name()) == preview)
        {
            return Err(ErrorKind::ChangeBusy(change.id().to_string()).into());
        }
    }
    Ok(())
}

pub(crate) async fn ensure_dependency_available(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    dependency: &concept::dependency::Dependency,
) -> Result<()> {
    for change in infra::change::list(docker, key, true).await? {
        if matches!(change.target(), concept::change::Target::PostgreSQL { dependency: target, .. } if target == dependency.reference())
            || dependency
                .local_app()
                .as_ref()
                .is_some_and(|app| change.occupies(app))
        {
            return Err(ErrorKind::ChangeBusy(change.id().to_string()).into());
        }
    }
    Ok(())
}

async fn available(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    change: &concept::change::Change,
) -> Result<()> {
    for current in infra::change::list(docker, key, true).await? {
        let same_target = match (change.target(), current.target()) {
            (
                concept::change::Target::PostgreSQL { dependency: a, .. },
                concept::change::Target::PostgreSQL { dependency: b, .. },
            ) => a == b,
            _ => false,
        };
        if same_target || change.apps().iter().any(|app| current.occupies(app.name())) {
            return Err(ErrorKind::ChangeBusy(current.id().to_string()).into());
        }
    }
    Ok(())
}

async fn checks(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    references: &[concept::dependency::Reference],
    own: Option<&concept::name::Name>,
) -> Result<Vec<concept::name::Name>> {
    let mut names = BTreeSet::new();
    if let Some(own) = own {
        names.insert(own.clone());
    }
    for reference in references {
        for app in infra::app::dependency_consumers(docker, key, reference).await? {
            names.insert(app.name().clone());
        }
    }
    let mut selected = Vec::new();
    for name in names {
        if infra::app::xhealth::load(docker, &name)
            .await?
            .is_some_and(|health| health.get().guardian())
        {
            selected.push(name);
        }
    }
    Ok(selected)
}

pub(crate) async fn submit_app(
    docker: &infra::docker::Docker,
    key: &Arc<MasterKey>,
    releases: &infra::release::Catalog,
    app: &concept::app::App,
    changes: concept::app::Changes,
    operator: concept::app::Operator,
) -> Result<Option<Uuid>> {
    let dependencies = infra::dependency::list(docker, None).await?;
    let mut references = vec![concept::dependency::Reference::App(app.name().clone())];
    references.extend(
        dependencies
            .iter()
            .filter(|dependency| dependency.local_app().as_ref() == Some(app.name()))
            .map(|dependency| {
                concept::dependency::Reference::PostgreSQL(dependency.reference().clone())
            }),
    );
    let selected = checks(docker, key, &references, Some(app.name())).await?;
    if selected.is_empty() {
        return Ok(None);
    }
    let mut apps: Vec<_> = selected
        .into_iter()
        .map(|name| concept::change::App::new(name, true, None))
        .collect();
    apps.push(concept::change::App::new(app.name().clone(), false, None));
    let change = concept::change::Change::new(
        concept::change::Target::App {
            name: app.name().clone(),
            changes: Box::new(changes),
        },
        apps,
        operator,
        Timestamp::now(),
    );
    available(docker, key, &change).await?;
    validate_previews(docker, key, &change).await?;
    let guard = acquire(change.id())?;
    infra::change::create(docker, key, &change).await?;
    let id = change.id();
    spawn(docker, key, releases, change, None, guard);
    Ok(Some(id))
}

#[expect(clippy::too_many_arguments)]
pub(crate) async fn create_sql(
    docker: &infra::docker::Docker,
    key: &Arc<MasterKey>,
    releases: &infra::release::Catalog,
    dependency: concept::name::Name,
    sql: cipher::Secret,
    size: Option<String>,
    operator: concept::app::Operator,
    now: Timestamp,
) -> Result<concept::change::Change> {
    if sql.reveal().trim().is_empty() || sql.reveal().contains('\0') {
        return Err(ErrorKind::InvalidBody(
            "SQL must be nonempty and cannot contain NUL".to_owned(),
        )
        .into());
    }
    let _admission = admission().await;
    let source = infra::dependency::load(docker, &dependency)
        .await?
        .ok_or(ErrorKind::SecretNotFound)?
        .into_inner();
    if let Some(size) = &size {
        match source.provider() {
            concept::dependency::Provider::Local => {
                return Err(ErrorKind::FieldUnsupported("size".to_owned()).into());
            }
            concept::dependency::Provider::Neon
                if !size
                    .parse::<f64>()
                    .is_ok_and(|value| value.is_finite() && value > 0.0) =>
            {
                return Err(ErrorKind::InvalidBody(
                    "Neon compute size must be a positive number".to_owned(),
                )
                .into());
            }
            concept::dependency::Provider::PlanetScale => {
                let sizes = super::dependency::planetscale_sizes(Some(&source))?;
                if !sizes.iter().any(|sku| sku.size == *size) {
                    return Err(ErrorKind::InvalidBody(
                        "unsupported PlanetScale rehearsal size".to_owned(),
                    )
                    .into());
                }
            }
            concept::dependency::Provider::Neon => {}
        }
    }
    let reference = concept::dependency::Reference::PostgreSQL(source.reference().clone());
    let mut apps: Vec<_> = checks(docker, key, &[reference], None)
        .await?
        .into_iter()
        .map(|name| concept::change::App::new(name, true, None))
        .collect();
    if let Some(app) = source.local_app() {
        apps.push(concept::change::App::new(app.clone(), false, None));
    }
    let change = concept::change::Change::new(
        concept::change::Target::PostgreSQL {
            dependency: source.reference().clone(),
            sql,
            size,
        },
        apps,
        operator,
        now,
    );
    available(docker, key, &change).await?;
    validate_previews(docker, key, &change).await?;
    let guard = acquire(change.id())?;
    infra::change::create(docker, key, &change).await?;
    infra::event::emit(
        Kind::ChangeSubmitted,
        &change.id().to_string(),
        change.target().name().as_str(),
    );
    spawn(docker, key, releases, change.clone(), None, guard);
    Ok(change)
}

async fn preview_apps(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    change: &concept::change::Change,
) -> Result<Vec<concept::app::App>> {
    if !change.has_checks() {
        return Ok(Vec::new());
    }
    let mut apps = Vec::new();
    for participant in change.apps() {
        let name = participant.name();
        let mut app = super::app::get(docker, key, name).await?;
        if let concept::change::Target::App {
            name: target,
            changes,
        } = change.target()
            && target == name
        {
            changes.apply(&mut app)?;
        }
        apps.push(app);
    }
    Ok(apps)
}

async fn validate_previews(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    change: &concept::change::Change,
) -> Result<()> {
    for app in preview_apps(docker, key, change).await? {
        super::volume::require_guardian(docker, &app).await?;
    }
    Ok(())
}

async fn sources(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    change: &concept::change::Change,
) -> Result<Vec<concept::dependency::Dependency>> {
    match change.target() {
        concept::change::Target::PostgreSQL { dependency, .. } => Ok(vec![
            infra::dependency::load(docker, dependency.name())
                .await?
                .filter(|source| source.get().reference() == dependency)
                .ok_or(ErrorKind::SecretNotFound)?
                .into_inner(),
        ]),
        concept::change::Target::App { name, .. } => {
            let all = infra::dependency::list(docker, None).await?;
            let mut used = HashSet::new();
            for app in preview_apps(docker, key, change).await? {
                for (_, binding) in app.dependency_env() {
                    if let concept::dependency::Reference::PostgreSQL(reference) =
                        binding.dependency()
                    {
                        used.insert(reference.id().clone());
                    }
                }
            }
            Ok(all
                .into_iter()
                .filter(|source| {
                    source.local_app().as_ref() == Some(name)
                        && used.contains(source.reference().id())
                })
                .collect())
        }
    }
}

fn spawn(
    docker: &infra::docker::Docker,
    key: &Arc<MasterKey>,
    releases: &infra::release::Catalog,
    change: concept::change::Change,
    action: Option<concept::change::Action>,
    guard: Guard,
) {
    let docker = docker.clone();
    let key = Arc::clone(key);
    let image = releases.list().and_then(|snapshot| {
        snapshot
            .releases()
            .get(&concept::release::Component::CdeliveryPostgres)
            .map(|release| release.image().clone())
    });
    let actor = infra::event::ACTOR.try_with(Clone::clone).ok();
    let execution = async move {
        let _guard = guard;
        let id = change.id();
        let result = match action {
            Some(action) => perform_action(&docker, &key, change, action, image.as_ref()).await,
            None => run(&docker, &key, change, image.as_ref()).await,
        };
        if let Err(error) = result {
            tracing::error!(error = ?error, "change {id} execution failed");
            let failure = async {
                let mut current = infra::change::load(&docker, &key, id)
                    .await?
                    .ok_or(ErrorKind::ChangeNotFound)?;
                if !current.get().active() {
                    return Ok(());
                }
                let diagnostic = error.chain(Locale::En);
                current.get_mut().fail(diagnostic.clone());
                infra::change::save(&docker, &key, &current).await?;
                infra::event::HUB.raise(
                    Kind::ChangeFailed,
                    &id.to_string(),
                    &format!("{}: {diagnostic}", current.get().target().name()),
                    Some(SignedDuration::ZERO),
                );
                Ok::<_, crate::error::Error>(())
            }
            .await;
            if let Err(error) = failure {
                tracing::error!(error = ?error, "change {id} failure could not be saved");
            }
        }
    };
    tokio::spawn(async move {
        if let Some(actor) = actor {
            infra::event::ACTOR.scope(actor, execution).await;
        } else {
            execution.await;
        }
    });
}

#[expect(clippy::too_many_lines)]
async fn run(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    mut change: concept::change::Change,
    image: Option<&concept::image::Reference>,
) -> Result<()> {
    let dependencies = sources(docker, key, &change).await?;
    if !dependencies.is_empty() {
        infra::app::postgres::create(docker, &change, &dependencies, image).await?;
    }
    if change.has_checks() {
        let planetscale_sizes = if dependencies
            .iter()
            .any(|dependency| dependency.provider() == concept::dependency::Provider::PlanetScale)
        {
            infra::dependency::sizes::list()
        } else {
            None
        };
        let mut connections = Vec::new();
        for dependency in &dependencies {
            let host = dependency
                .local_app()
                .as_ref()
                .map(|app| infra::app::preview::name(change.id(), app));
            let size = match change.target() {
                concept::change::Target::PostgreSQL { size, .. } => size.clone(),
                concept::change::Target::App { .. } => None,
            };
            let planetscale_sizes =
                if dependency.provider() == concept::dependency::Provider::PlanetScale {
                    planetscale_sizes.clone()
                } else {
                    None
                };
            let response = infra::app::postgres::request(
                key,
                &change,
                cdelivery_wire::postgres::Command::Prepare {
                    dependency: dependency.reference().name().to_string(),
                    name: change.branch_name(),
                    host,
                    size,
                    planetscale_sizes,
                },
            )
            .await?;
            let cdelivery_wire::postgres::Response::Prepared { dsn } = response else {
                return Err(ErrorKind::ChangeExecution(
                    "executor returned an unexpected preparation result".to_owned(),
                )
                .into());
            };
            let secret = infra::app::postgres::create_connection(
                docker,
                &change,
                dependency.reference(),
                dsn.reveal(),
            )
            .await?;
            connections.push((dependency.reference().id().clone(), secret));
        }
        let apps = preview_apps(docker, key, &change).await?;
        let primary = match change.target() {
            concept::change::Target::App { name, .. } => Some(name),
            concept::change::Target::PostgreSQL { .. } => dependencies
                .first()
                .and_then(|dependency| dependency.local_app().as_ref()),
        };
        if let Some(primary) = primary {
            let app = apps
                .iter()
                .find(|app| app.name() == primary)
                .ok_or(ErrorKind::AppNotFound)?;
            infra::app::preview::create(docker, key, &change, app, &connections).await?;
            infra::app::preview::wait_ready(docker, change.id(), app.name()).await?;
        }
        if matches!(change.target(), concept::change::Target::App { .. }) {
            for dependency in &dependencies {
                let response = infra::app::postgres::request(
                    key,
                    &change,
                    cdelivery_wire::postgres::Command::Execute {
                        dependency: dependency.reference().name().to_string(),
                        target: cdelivery_wire::postgres::Target::Preview,
                        sql: cipher::Secret::new("SELECT 1".to_owned()),
                    },
                )
                .await?;
                if !matches!(response, cdelivery_wire::postgres::Response::Done) {
                    return Err(ErrorKind::ChangeExecution(
                        "executor returned an unexpected readiness result".to_owned(),
                    )
                    .into());
                }
            }
        } else {
            execute_sql(key, &change, cdelivery_wire::postgres::Target::Preview).await?;
        }
        for app in &apps {
            if Some(app.name()) != primary {
                infra::app::preview::create(docker, key, &change, app, &connections).await?;
            }
        }
        for result in futures_util::future::join_all(
            apps.iter()
                .map(|app| infra::app::preview::wait_ready(docker, change.id(), app.name())),
        )
        .await
        {
            result?;
        }
        change = update(docker, key, change.id(), |current| {
            current.advance(concept::change::Stage::Check);
        })
        .await?;
        let mut submitted = Vec::new();
        let mut results = BTreeMap::new();
        for participant in change.apps().iter().filter(|app| app.check()) {
            let name = participant.name();
            let submission = async {
                let app = apps
                    .iter()
                    .find(|app| app.name() == name)
                    .ok_or(ErrorKind::AppNotFound)?;
                let health = infra::app::xhealth::load(docker, name)
                    .await?
                    .ok_or(ErrorKind::XHealthNotEnabled)?
                    .into_inner();
                infra::app::preview::create_check(docker, &change, app, &health, &connections)
                    .await?;
                Ok::<_, crate::error::Error>(config::expiry(Timestamp::now(), health.interval()))
            }
            .await;
            match submission {
                Ok(deadline) => submitted.push((name.clone(), deadline)),
                Err(error) => {
                    results.insert(name.clone(), Err(error));
                    break;
                }
            }
        }
        let finished = futures_util::future::join_all(submitted.iter().map(|(name, deadline)| {
            infra::app::preview::check(docker, change.id(), name, *deadline)
        }))
        .await;
        for ((name, _), result) in submitted.into_iter().zip(finished) {
            results.insert(name, result);
        }
        let results = results
            .into_iter()
            .map(|(name, result)| {
                let result = match result {
                    Ok(()) => concept::change::CheckResult::Passed,
                    Err(error) => {
                        tracing::error!(error = ?error, "change {} check {name} failed", change.id());
                        concept::change::CheckResult::Failed {
                            error: error.chain(Locale::En),
                        }
                    }
                };
                (name, result)
            })
            .collect();
        change = update(docker, key, change.id(), |current| {
            current.finish_checks(results);
        })
        .await?;
        if change.stage() == concept::change::Stage::Check {
            let diagnostic = change
                .apps()
                .iter()
                .filter_map(|app| match app.result() {
                    Some(concept::change::CheckResult::Failed { error }) => {
                        Some(format!("{}: {error}", app.name()))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            infra::event::HUB.raise(
                Kind::ChangeFailed,
                &change.id().to_string(),
                &format!("{}: {diagnostic}", change.target().name()),
                Some(SignedDuration::ZERO),
            );
            return Ok(());
        }
        clean_rehearsal(docker, key, &change, &dependencies, image).await?;
    }
    if matches!(change.target(), concept::change::Target::App { .. }) {
        infra::app::postgres::delete(docker, &change).await?;
    }
    apply(docker, key, &change).await
}

async fn execute_sql(
    key: &MasterKey,
    change: &concept::change::Change,
    target: cdelivery_wire::postgres::Target,
) -> Result<()> {
    if let concept::change::Target::PostgreSQL {
        dependency, sql, ..
    } = change.target()
    {
        let result = infra::app::postgres::request(
            key,
            change,
            cdelivery_wire::postgres::Command::Execute {
                dependency: dependency.name().to_string(),
                target,
                sql: sql.clone(),
            },
        )
        .await?;
        if !matches!(result, cdelivery_wire::postgres::Response::Done) {
            return Err(ErrorKind::ChangeExecution(
                "executor returned an unexpected SQL result".to_owned(),
            )
            .into());
        }
    }
    Ok(())
}

async fn clean_rehearsal(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    change: &concept::change::Change,
    dependencies: &[concept::dependency::Dependency],
    image: Option<&concept::image::Reference>,
) -> Result<()> {
    for app in preview_apps(docker, key, change).await?.iter().rev() {
        infra::app::preview::delete(docker, key, change.id(), app).await?;
    }
    if change.has_checks() {
        for dependency in dependencies {
            if dependency.provider() != concept::dependency::Provider::Local {
                infra::app::postgres::create(docker, change, dependencies, image).await?;
                let result = infra::app::postgres::request(
                    key,
                    change,
                    cdelivery_wire::postgres::Command::Cleanup {
                        dependency: dependency.reference().name().to_string(),
                        name: change.branch_name(),
                    },
                )
                .await?;
                if !matches!(result, cdelivery_wire::postgres::Response::Done) {
                    return Err(ErrorKind::ChangeExecution(
                        "executor returned an unexpected cleanup result".to_owned(),
                    )
                    .into());
                }
            }
            infra::app::postgres::delete_connection(docker, change, dependency.reference()).await?;
        }
    }
    Ok(())
}

async fn apply(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    change: &concept::change::Change,
) -> Result<()> {
    update(docker, key, change.id(), |current| {
        current.advance(concept::change::Stage::Apply);
    })
    .await?;
    match change.target() {
        concept::change::Target::App { name, changes } => {
            let mut app = infra::app::load(docker, key, name)
                .await?
                .ok_or(ErrorKind::AppNotFound)?;
            changes.apply(app.get_mut())?;
            app.get_mut().record_operator(change.initiator());
            infra::app::save(docker, key, &app).await?;
        }
        concept::change::Target::PostgreSQL { .. } => {
            execute_sql(key, change, cdelivery_wire::postgres::Target::Production).await?;
            update(docker, key, change.id(), |current| {
                current.advance(concept::change::Stage::Finish);
            })
            .await?;
            infra::app::postgres::delete(docker, change).await?;
        }
    }
    update(docker, key, change.id(), concept::change::Change::complete).await?;
    infra::event::HUB.clear(
        Kind::ChangeFailed,
        &change.id().to_string(),
        "Change completed.",
    );
    infra::event::emit(
        Kind::ChangeCompleted,
        &change.id().to_string(),
        change.target().name().as_str(),
    );
    Ok(())
}

pub(crate) async fn act(
    docker: &infra::docker::Docker,
    key: &Arc<MasterKey>,
    releases: &infra::release::Catalog,
    id: Uuid,
    action: concept::change::Action,
) -> Result<()> {
    let guard = acquire(id)?;
    let change = get(docker, key, id).await?;
    change.validate_action(action)?;
    spawn(docker, key, releases, change, Some(action), guard);
    Ok(())
}

async fn perform_action(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    change: concept::change::Change,
    action: concept::change::Action,
    image: Option<&concept::image::Reference>,
) -> Result<()> {
    if action == concept::change::Action::Complete {
        update(docker, key, change.id(), |current| {
            current.advance(concept::change::Stage::Finish);
        })
        .await?;
        infra::app::postgres::delete(docker, &change).await?;
    } else {
        infra::app::postgres::delete(docker, &change).await?;
        let dependencies = sources(docker, key, &change).await?;
        clean_rehearsal(docker, key, &change, &dependencies, image).await?;
        infra::app::postgres::delete(docker, &change).await?;
        if action == concept::change::Action::Apply {
            if matches!(change.target(), concept::change::Target::PostgreSQL { .. }) {
                infra::app::postgres::create(docker, &change, &dependencies, image).await?;
            }
            return apply(docker, key, &change).await;
        }
    }
    update(docker, key, change.id(), |current| match action {
        concept::change::Action::Complete => current.complete(),
        concept::change::Action::Close => current.close(),
        concept::change::Action::Apply => {}
    })
    .await?;
    infra::event::HUB.clear(
        Kind::ChangeFailed,
        &change.id().to_string(),
        "Change resolved.",
    );
    let kind = if action == concept::change::Action::Close {
        Kind::ChangeClosed
    } else {
        Kind::ChangeCompleted
    };
    infra::event::emit(
        kind,
        &change.id().to_string(),
        change.target().name().as_str(),
    );
    Ok(())
}
