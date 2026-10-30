use cipher::MasterKey;
use insight_wire::event::Kind;
use jiff::{SignedDuration, Timestamp};

use crate::concept;
use crate::error::{ErrorKind, Result};
use crate::infra;
use crate::locale::Locale;

async fn health_config(
    docker: &infra::docker::Docker,
    name: &concept::name::Name,
) -> Result<Option<concept::xhealth::Spec>> {
    match infra::app::xhealth::load(docker, name).await {
        Ok(unit) => Ok(unit.map(concept::versioned::Versioned::into_inner)),
        Err(e) if matches!(e.kind(), ErrorKind::Spec(_)) => {
            tracing::warn!(error = ?e, "guardian treats undecodable xhealth config for app {name} as missing");
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

async fn candidate(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    guardian: &concept::guardian::Guardian,
) -> Result<concept::app::App> {
    let mut app = super::found(
        infra::app::load(docker, key, guardian.name()),
        ErrorKind::AppNotFound,
    )
    .await?;
    guardian.changes().apply(&mut app)?;
    Ok(app)
}

async fn run_check(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    name: &concept::name::Name,
    guardian: &concept::guardian::Guardian,
    health: Option<&concept::xhealth::Spec>,
    shadow_version: u64,
    check_version: Option<u64>,
) -> Result<()> {
    let Some(spec) = health else {
        return Ok(());
    };
    let candidate = candidate(docker, key, guardian).await?;
    infra::app::guardian::run_check(
        docker,
        name,
        &candidate,
        spec,
        shadow_version,
        check_version,
    )
    .await
}

pub(crate) async fn get(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    name: &concept::name::Name,
    now: Timestamp,
) -> Result<concept::guardian::Status> {
    let guardian = infra::app::guardian::load(docker, key, name)
        .await?
        .ok_or(ErrorKind::GuardianNotFound)?
        .into_inner();
    let health = health_config(docker, name).await?;
    let deadline = health.as_ref().map(concept::xhealth::Spec::interval);
    let snapshot = infra::app::guardian::observe(docker, name, now)
        .await?
        .ok_or(ErrorKind::GuardianNotFound)?;
    Ok(concept::guardian::Status::new(
        guardian, snapshot, deadline, now,
    ))
}

async fn tick(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    name: &concept::name::Name,
    now: Timestamp,
) -> Result<()> {
    let Some(mut guardian) = infra::app::guardian::load(docker, key, name).await? else {
        infra::app::guardian::delete_check(docker, name).await?;
        return Ok(());
    };
    let health = health_config(docker, name).await?;
    let deadline = health.as_ref().map(concept::xhealth::Spec::interval);
    let Some(snapshot) = infra::app::guardian::observe(docker, name, now).await? else {
        return Ok(());
    };
    let action =
        concept::guardian::next_action(guardian.get().verdict().as_ref(), &snapshot, deadline, now);
    match action {
        concept::guardian::Action::Stop {
            error,
            check_version,
            ..
        } => {
            infra::event::HUB.raise(
                Kind::GuardianFailed,
                name.as_str(),
                &error,
                Some(SignedDuration::ZERO),
            );
            infra::app::guardian::stop(docker, name, guardian.version(), check_version).await
        }
        concept::guardian::Action::Wait(_) => Ok(()),
        concept::guardian::Action::RunCheck {
            shadow_version,
            check_version,
        } => {
            run_check(
                docker,
                key,
                name,
                guardian.get(),
                health.as_ref(),
                shadow_version,
                check_version,
            )
            .await
        }
        concept::guardian::Action::RecordVerdict(verdict) => {
            guardian.get_mut().conclude(verdict)?;
            infra::app::guardian::save(docker, key, &guardian).await?;
            if let Some(concept::guardian::Verdict::Failed { error, .. }) =
                guardian.get().verdict().as_ref()
            {
                infra::event::HUB.raise(
                    Kind::GuardianFailed,
                    name.as_str(),
                    error,
                    Some(SignedDuration::ZERO),
                );
            }
            Ok(())
        }
        concept::guardian::Action::Promote => {
            let outcome = promote(docker, key, guardian.into_inner()).await;
            if let Err(e) = outcome
                && !e.is_conflict()
            {
                infra::event::HUB.raise(
                    Kind::GuardianFailed,
                    name.as_str(),
                    &format!("promote failed: {}", e.chain(Locale::En)),
                    Some(SignedDuration::ZERO),
                );
                tracing::warn!(error = ?e, "guardian promote failed for {name}");
            }
            Ok(())
        }
    }
}

async fn promote(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    pending: concept::guardian::Guardian,
) -> Result<()> {
    let name = pending.name().clone();
    let Some(mut live) = infra::app::load(docker, key, &name).await? else {
        return infra::app::guardian::delete(docker, &name).await;
    };
    pending.changes().apply(live.get_mut())?;
    if live.changed() {
        live.get_mut().record_operator(pending.initiator());
    }
    infra::app::save(docker, key, &live).await?;
    infra::app::guardian::delete(docker, &name).await?;
    infra::event::HUB.clear(Kind::GuardianFailed, name.as_str());
    let note = match pending.verdict() {
        Some(concept::guardian::Verdict::Skipped { .. }) => "check skipped by operator",
        _ => "",
    };
    infra::event::emit(Kind::GuardianPromoted, name.as_str(), note);
    Ok(())
}

pub(crate) async fn retry(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    name: &concept::name::Name,
) -> Result<()> {
    let mut guardian = infra::app::guardian::load(docker, key, name)
        .await?
        .ok_or(ErrorKind::GuardianNotFound)?;
    guardian.get_mut().reopen()?;
    let candidate = candidate(docker, key, guardian.get()).await?;
    infra::app::guardian::redeploy(docker, key, &guardian, &candidate).await?;
    infra::event::emit(Kind::GuardianRetried, name.as_str(), "");
    Ok(())
}

pub(crate) async fn skip(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    name: &concept::name::Name,
    now: Timestamp,
) -> Result<()> {
    let mut guardian = infra::app::guardian::load(docker, key, name)
        .await?
        .ok_or(ErrorKind::GuardianNotFound)?;
    guardian.get_mut().skip(now)?;
    infra::app::guardian::save(docker, key, &guardian).await?;
    infra::event::emit(Kind::GuardianSkipped, name.as_str(), "");
    infra::event::HUB.dismiss(Kind::GuardianFailed, name.as_str());
    Ok(())
}

pub(crate) async fn cancel(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    name: &concept::name::Name,
    now: Timestamp,
) -> Result<()> {
    match infra::app::guardian::load(docker, key, name).await {
        Ok(None) => return Ok(()),
        Ok(Some(guardian)) => guardian.get().cancel(now)?,
        Err(e) if matches!(e.kind(), ErrorKind::Spec(_) | ErrorKind::Cipher(_)) => {
            tracing::warn!(error = ?e, "guardian cancel discards undecodable record for app {name}");
        }
        Err(e) => return Err(e),
    }
    infra::app::guardian::delete(docker, name).await?;
    infra::event::emit(Kind::GuardianCancelled, name.as_str(), "");
    infra::event::HUB.dismiss(Kind::GuardianFailed, name.as_str());
    Ok(())
}

pub(crate) async fn sweep(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    now: Timestamp,
) -> Vec<(String, String)> {
    super::sweep_units(
        infra::app::guardian::owners(docker).await,
        "guardian",
        Kind::GuardianFailed,
        ToString::to_string,
        |_| None,
        |name| async move { tick(docker, key, &name, now).await.map(|()| false) },
    )
    .await
    .1
}
