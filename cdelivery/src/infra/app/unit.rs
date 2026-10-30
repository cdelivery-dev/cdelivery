use std::collections::HashMap;

use docker::types::{Filter, ServiceSpec};
use strum::{Display, EnumString};

use crate::concept;
use crate::error::{Result, SpecProblem};

pub(super) const LABEL_APP: &str = "dev.cdelivery.app_name";
pub(crate) const LABEL_KIND: &str = "dev.cdelivery.app_service_kind";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, EnumString)]
pub(crate) enum Kind {
    App,
    XHealth,
    XJob,
    GuardianApp,
    GuardianCheck,
}

impl Kind {
    pub(super) fn filter(self) -> Filter {
        Filter::new().label(format!("{LABEL_KIND}={self}"))
    }

    pub(super) fn for_app(self, app: &concept::name::Name) -> Filter {
        filter(app).label(format!("{LABEL_KIND}={self}"))
    }
}

pub(super) fn filter(app: &concept::name::Name) -> Filter {
    Filter::new().label(format!("{LABEL_APP}={app}"))
}

pub(super) fn labels(app: &concept::name::Name, kind: Kind) -> HashMap<String, String> {
    HashMap::from([
        (LABEL_APP.to_owned(), app.to_string()),
        (LABEL_KIND.to_owned(), kind.to_string()),
    ])
}

pub(crate) fn decode(spec: &ServiceSpec) -> Result<(concept::name::Name, Kind)> {
    let app: concept::name::Name = super::require(&spec.labels, LABEL_APP, &spec.name)?
        .parse()
        .map_err(|_| super::bad_label(&spec.name, LABEL_APP))?;
    let kind = super::require(&spec.labels, LABEL_KIND, &spec.name)?
        .parse()
        .map_err(|_| super::bad_label(&spec.name, LABEL_KIND))?;
    let expected = match kind {
        Kind::App => app.to_string(),
        Kind::XHealth => super::xhealth::service_name(&app),
        Kind::XJob => super::xjob::service_name(&app, &super::xjob::job_name(spec)?),
        Kind::GuardianApp => super::guardian::shadow_name(&app),
        Kind::GuardianCheck => super::guardian::check_name(&app),
    };
    if spec.name != expected {
        return Err(SpecProblem::BadName(spec.name.clone()).into());
    }
    Ok((app, kind))
}

pub(super) fn owner(spec: &ServiceSpec, expected: Kind) -> Result<concept::name::Name> {
    let (app, kind) = decode(spec)?;
    if kind != expected {
        return Err(super::bad_label(&spec.name, LABEL_KIND));
    }
    Ok(app)
}
