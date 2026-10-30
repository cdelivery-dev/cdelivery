use std::collections::HashMap;

use jiff::Timestamp;

use crate::{concept, operation};

pub(super) struct Dashboard {
    pub(super) user_name: String,
    pub(super) initials: String,
    pub(super) groups: Vec<GroupLink>,
    pub(super) can_create: bool,
    pub(super) can_create_app: bool,
    pub(super) can_manage_settings: bool,
    pub(super) location: Location,
}

pub(super) enum Location {
    Overview,
    Insight,
    Pingress,
    Volume,
    Nodes,
    Groups,
    Group(String),
    App(String),
    AppCreate(String),
}

pub(super) struct GroupLink {
    pub(super) name: String,
    pub(super) description: Option<String>,
    pub(super) apps: Vec<String>,
    pub(super) can_write: bool,
}

impl Dashboard {
    pub(super) async fn load(
        app: &crate::boot::App,
        access: &concept::permission::Access,
        location: Location,
    ) -> crate::error::Result<Self> {
        let now = Timestamp::now();
        let (groups, apps) = tokio::try_join!(
            operation::execute::<operation::group::List>(
                app,
                access,
                operation::input::Empty {},
                now,
            ),
            operation::execute::<operation::app::List>(
                app,
                access,
                operation::input::Empty {},
                now,
            ),
        )?;
        let can_write = access.allows(concept::permission::Requirement::Admin);
        let mut apps_by_group = HashMap::<String, Vec<String>>::new();
        for app in apps.apps {
            apps_by_group.entry(app.group).or_default().push(app.name);
        }
        let groups = groups
            .groups
            .into_iter()
            .map(|group| {
                let apps = apps_by_group.remove(&group.name).unwrap_or_default();
                GroupLink {
                    apps,
                    name: group.name,
                    description: group.description,
                    can_write,
                }
            })
            .collect();
        let user_name = access.actor().to_string();
        let initials = initials(&user_name);
        Ok(Self {
            user_name,
            initials,
            groups,
            can_create: can_write,
            can_create_app: access.allows(concept::permission::Requirement::Operator),
            can_manage_settings: can_write,
            location,
        })
    }
}

pub(super) fn initials(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|part| part.chars().next())
        .take(2)
        .flat_map(char::to_uppercase)
        .collect()
}
