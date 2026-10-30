use std::collections::{BTreeMap, BTreeSet};

use getset::{CopyGetters, Getters};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use totp_rs::Secret;
use uuid::Uuid;

use crate::locale::Locale;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) enum Role {
    Reader,
    Operator,
    Admin,
}

const _: () = assert!(
    (Role::Reader as u8) < (Role::Operator as u8) && (Role::Operator as u8) < (Role::Admin as u8)
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, CopyGetters)]
pub(crate) struct ApiKey {
    #[getset(get_copy = "pub(crate)")]
    id: Uuid,
    #[getset(get = "pub(crate)")]
    name: String,
    #[getset(get_copy = "pub(crate)")]
    created_at: Timestamp,
    #[getset(get_copy = "pub(crate)")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires_at: Option<Timestamp>,
    #[getset(get_copy = "pub(crate)")]
    role: Role,
    index: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, CopyGetters, Default)]
pub(crate) struct Preferences {
    #[serde(default)]
    #[getset(get_copy = "pub(crate)")]
    language: Option<Locale>,
    #[serde(default)]
    #[getset(get_copy = "pub(crate)")]
    theme: crate::theme::Theme,
    #[serde(default)]
    #[getset(get_copy = "pub(crate)")]
    alert_email: bool,
    #[serde(default)]
    #[getset(get = "pub(crate)")]
    alerts: BTreeSet<super::message::Im>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Getters, CopyGetters)]
pub(crate) struct User {
    #[getset(get_copy = "pub(crate)")]
    id: Uuid,
    #[getset(get = "pub(crate)")]
    first_name: String,
    #[getset(get = "pub(crate)")]
    last_name: String,
    #[getset(get = "pub(crate)")]
    email: String,
    #[getset(get = "pub(crate)")]
    password_hash: String,
    #[getset(get = "pub(crate)")]
    api_keys: Vec<ApiKey>,
    #[getset(get = "pub(crate)")]
    preferences: Preferences,
    #[getset(get = "pub(crate)")]
    im: BTreeMap<super::message::Im, String>,
    #[getset(get_copy = "pub(crate)")]
    role: Role,
    #[getset(get = "pub(crate)")]
    totp_secret: Option<Secret>,
}

impl std::fmt::Debug for User {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("User")
            .field("id", &self.id)
            .field("first_name", &self.first_name)
            .field("last_name", &self.last_name)
            .field("email", &self.email)
            .field("password_hash", &"<redacted>")
            .field("api_keys", &self.api_keys.len())
            .field("preferences", &self.preferences)
            .field("im", &self.im)
            .field("role", &self.role)
            .field("totp_enabled", &self.totp_secret.is_some())
            .finish()
    }
}

impl User {
    pub(crate) fn new(
        first_name: String,
        last_name: String,
        email: String,
        password_hash: String,
        role: Role,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            first_name,
            last_name,
            email,
            password_hash,
            api_keys: Vec::new(),
            preferences: Preferences::default(),
            im: BTreeMap::new(),
            role,
            totp_secret: None,
        }
    }

    pub(crate) fn name(&self) -> String {
        format!("{} {}", self.first_name, self.last_name)
    }

    #[expect(clippy::option_option)]
    pub(crate) fn update_preferences(
        &mut self,
        language: Option<Option<Locale>>,
        theme: Option<crate::theme::Theme>,
        alert_email: Option<bool>,
        alerts: Option<BTreeSet<super::message::Im>>,
    ) {
        self.preferences.language = language.unwrap_or(self.preferences.language);
        self.preferences.theme = theme.unwrap_or(self.preferences.theme);
        self.preferences.alert_email = alert_email.unwrap_or(self.preferences.alert_email);
        if let Some(alerts) = alerts {
            self.preferences.alerts = alerts;
        }
    }

    pub(crate) fn set_im(&mut self, im: super::message::Im, id: Option<String>) {
        match id {
            Some(id) => {
                self.im.insert(im, id);
            }
            None => {
                self.im.remove(&im);
            }
        }
    }

    pub(crate) fn rename(&mut self, first_name: Option<String>, last_name: Option<String>) {
        if let Some(first_name) = first_name {
            self.first_name = first_name;
        }
        if let Some(last_name) = last_name {
            self.last_name = last_name;
        }
    }

    pub(crate) fn set_email(&mut self, email: String) {
        self.email = email;
    }

    pub(crate) fn change_password(&mut self, password_hash: String) {
        self.password_hash = password_hash;
    }

    pub(crate) fn set_role(&mut self, role: Role) {
        self.role = role;
    }

    pub(crate) fn totp_enabled(&self) -> bool {
        self.totp_secret.is_some()
    }

    pub(crate) fn enable_totp(&mut self, secret: Secret) {
        self.totp_secret = Some(secret);
    }

    pub(crate) fn disable_totp(&mut self) {
        self.totp_secret = None;
    }

    pub(crate) fn add_api_key(
        &mut self,
        name: String,
        index: String,
        created_at: Timestamp,
        expires_at: Option<Timestamp>,
        role: Role,
    ) -> ApiKey {
        let key = ApiKey {
            id: Uuid::new_v4(),
            name,
            created_at,
            expires_at,
            role,
            index,
        };
        self.api_keys.push(key.clone());
        key
    }

    pub(crate) fn revoke_api_key(&mut self, id: Uuid) {
        self.api_keys.retain(|k| k.id != id);
    }

    pub(crate) fn api_key_by_index(&self, index: &str, now: Timestamp) -> Option<&ApiKey> {
        self.api_keys
            .iter()
            .find(|k| k.index == index && k.expires_at.is_none_or(|exp| exp > now))
    }

    pub(crate) fn effective_role(&self, api_key: Option<&ApiKey>) -> Role {
        api_key.map_or(self.role, |key| self.role.min(key.role))
    }
}

impl super::versioned::Aggregate for User {}
