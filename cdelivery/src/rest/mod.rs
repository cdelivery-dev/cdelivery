mod apikey;
mod app;
mod event;
pub(crate) mod extract;
mod group;
mod guardian;
mod insight;
mod instance;
mod node;
pub(crate) mod pingress;
pub(crate) mod response;
mod settings;
mod user;
mod volume;
mod xhealth;
mod xjob;

use std::sync::Arc;

use utoipa_axum::router::OpenApiRouter;

use crate::boot::App;
use crate::operation;

const TAG_APPS: &str = "Apps";
const TAG_GROUPS: &str = "Groups";
const TAG_XHEALTH: &str = "XHealth";
const TAG_XJOBS: &str = "XJobs";
const TAG_GUARDIAN: &str = "Guardian";
const TAG_NODES: &str = "Nodes";
const TAG_PINGRESS: &str = "Pingress";
const TAG_INSIGHT: &str = "Insight";
const TAG_EVENTS: &str = "Events";
const TAG_INSTANCE: &str = "Instance";
const TAG_USERS: &str = "Users";
const TAG_API_KEYS: &str = "API Keys";
const TAG_SETTINGS: &str = "Settings";
pub(crate) const API_TAGS: &[&str] = &[
    TAG_APPS,
    TAG_GROUPS,
    TAG_XHEALTH,
    TAG_XJOBS,
    TAG_GUARDIAN,
    TAG_NODES,
    TAG_PINGRESS,
    TAG_INSIGHT,
    TAG_EVENTS,
    TAG_INSTANCE,
    TAG_USERS,
    TAG_API_KEYS,
    TAG_SETTINGS,
];

pub(crate) trait Route: operation::catalog::Catalogued {
    const METHOD: &'static str;
    const PATH: &'static str;
}

pub(crate) struct RouteSpec {
    pub(crate) id: &'static str,
    pub(crate) method: &'static str,
    pub(crate) path: &'static str,
}

impl RouteSpec {
    const fn new<O: Route>() -> Self {
        Self {
            id: O::ID,
            method: O::METHOD,
            path: O::PATH,
        }
    }
}

macro_rules! routes {
    ($($operation:path),+ $(,)?) => {
        pub(crate) const ROUTES: &[RouteSpec] = &[
            $(RouteSpec::new::<$operation>(),)+
        ];
    };
}

operation::catalog::operations!(routes);

pub(crate) fn for_request(
    method: &http::Method,
    path: &str,
) -> Option<&'static operation::catalog::OperationSpec> {
    let method = if method == http::Method::HEAD {
        "GET"
    } else {
        method.as_str()
    };
    ROUTES
        .iter()
        .find(|route| route.method.eq_ignore_ascii_case(method) && route.path == path)
        .and_then(|route| operation::catalog::find(route.id))
}

pub(crate) fn protected_routes() -> OpenApiRouter<Arc<App>> {
    tagged(user::routes(), TAG_USERS)
        .merge(tagged(apikey::routes(), TAG_API_KEYS))
        .merge(tagged(group::routes(), TAG_GROUPS))
        .merge(tagged(app::routes(), TAG_APPS))
        .merge(tagged(xhealth::routes(), TAG_XHEALTH))
        .merge(tagged(xjob::routes(), TAG_XJOBS))
        .merge(tagged(guardian::routes(), TAG_GUARDIAN))
        .merge(tagged(node::routes(), TAG_NODES))
        .merge(tagged(volume::routes(), TAG_NODES))
        .merge(tagged(pingress::routes(), TAG_PINGRESS))
        .merge(tagged(event::routes(), TAG_EVENTS))
        .merge(tagged(insight::routes(), TAG_INSIGHT))
        .merge(tagged(instance::routes(), TAG_INSTANCE))
        .merge(tagged(settings::routes(), TAG_SETTINGS))
}

fn tagged<S>(mut routes: OpenApiRouter<S>, tag: &'static str) -> OpenApiRouter<S>
where
    S: Clone + Send + Sync + 'static,
{
    for item in routes.get_openapi_mut().paths.paths.values_mut() {
        for operation in [
            item.get.as_mut(),
            item.put.as_mut(),
            item.post.as_mut(),
            item.delete.as_mut(),
            item.options.as_mut(),
            item.head.as_mut(),
            item.patch.as_mut(),
            item.trace.as_mut(),
        ]
        .into_iter()
        .flatten()
        {
            operation
                .tags
                .get_or_insert_with(Vec::new)
                .push(tag.to_owned());
        }
    }
    routes
}

pub(crate) struct BinaryArtifact;

impl utoipa::PartialSchema for BinaryArtifact {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        utoipa::openapi::ObjectBuilder::new()
            .schema_type(utoipa::openapi::schema::Type::String)
            .format(Some(utoipa::openapi::SchemaFormat::KnownFormat(
                utoipa::openapi::KnownFormat::Binary,
            )))
            .into()
    }
}

impl utoipa::ToSchema for BinaryArtifact {}
