use std::collections::{BTreeMap, BTreeSet};

use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::concept;
use crate::locale::Locale;

#[derive(Serialize, ToSchema)]
pub(crate) struct ManagedUser {
    pub(crate) id: String,
    pub(crate) first_name: String,
    pub(crate) last_name: String,
    pub(crate) email: String,
    pub(crate) role: super::super::typ::Role,
    pub(crate) totp_enabled: bool,
}

impl From<concept::user::User> for ManagedUser {
    fn from(u: concept::user::User) -> Self {
        Self {
            id: u.id().to_string(),
            first_name: u.first_name().to_owned(),
            last_name: u.last_name().to_owned(),
            email: u.email().to_owned(),
            role: u.role().into(),
            totp_enabled: u.totp_enabled(),
        }
    }
}

#[derive(Serialize, ToSchema)]
pub(crate) struct SelfUser {
    pub(crate) id: String,
    pub(crate) first_name: String,
    pub(crate) last_name: String,
    pub(crate) email: String,
    pub(crate) role: super::super::typ::Role,
    pub(crate) totp_enabled: bool,
    pub(crate) preferences: Preferences,
}

impl From<concept::user::User> for SelfUser {
    fn from(u: concept::user::User) -> Self {
        Self {
            id: u.id().to_string(),
            first_name: u.first_name().to_owned(),
            last_name: u.last_name().to_owned(),
            email: u.email().to_owned(),
            role: u.role().into(),
            totp_enabled: u.totp_enabled(),
            preferences: u.preferences().into(),
        }
    }
}

#[derive(Serialize, ToSchema)]
pub(crate) struct Preferences {
    pub(crate) language: Option<Locale>,
    pub(crate) theme: crate::theme::Theme,
    pub(crate) alert_email: bool,
    pub(crate) alerts: BTreeSet<super::super::typ::Im>,
}

impl From<&concept::user::Preferences> for Preferences {
    fn from(preferences: &concept::user::Preferences) -> Self {
        Self {
            language: preferences.language(),
            theme: preferences.theme(),
            alert_email: preferences.alert_email(),
            alerts: preferences.alerts().iter().map(|im| (*im).into()).collect(),
        }
    }
}

#[derive(Serialize, ToSchema)]
pub(crate) struct ImBindings {
    pub(crate) bindings: BTreeMap<super::super::typ::Im, String>,
}

#[derive(Serialize, ToSchema)]
pub(crate) struct ImProfile {
    pub(crate) name: Option<String>,
    pub(crate) avatar: Option<String>,
}

#[macros_derive::validated_schema]
#[derive(Deserialize, ToSchema, Validate)]
pub(crate) struct CreateUserRequest {
    #[garde(length(min = 1, max = 64))]
    pub(crate) first_name: String,
    #[garde(length(min = 1, max = 64))]
    pub(crate) last_name: String,
    #[garde(email, length(max = 254))]
    pub(crate) email: String,
    #[garde(custom(super::super::validate::sha256_hex))]
    #[schema(pattern = "^[0-9a-f]{64}$")]
    pub(crate) password_sha256: String,
    #[garde(skip)]
    pub(crate) role: super::super::typ::Role,
}

#[macros_derive::validated_schema(patch)]
#[derive(Deserialize, ToSchema, Validate)]
pub(crate) struct UpdateUserRequest {
    #[garde(inner(length(min = 1, max = 64)))]
    pub(crate) first_name: Option<String>,
    #[garde(inner(length(min = 1, max = 64)))]
    pub(crate) last_name: Option<String>,
    #[garde(inner(email, length(max = 254)))]
    pub(crate) email: Option<String>,
    #[garde(skip)]
    pub(crate) role: Option<super::super::typ::Role>,
    #[garde(inner(custom(super::super::validate::sha256_hex)))]
    #[schema(pattern = "^[0-9a-f]{64}$")]
    pub(crate) password_sha256: Option<String>,
    #[garde(skip)]
    #[serde(default)]
    pub(crate) reset_totp: bool,
}

#[derive(Serialize, ToSchema)]
pub(crate) struct Users {
    pub(crate) users: Vec<ManagedUser>,
}

#[macros_derive::validated_schema(patch)]
#[derive(Deserialize, ToSchema, Validate)]
pub(crate) struct UpdateProfileRequest {
    #[garde(inner(length(min = 1, max = 64)))]
    pub(crate) first_name: Option<String>,
    #[garde(inner(length(min = 1, max = 64)))]
    pub(crate) last_name: Option<String>,
}

#[macros_derive::validated_schema]
#[derive(Deserialize, ToSchema, Validate)]
pub(crate) struct ChangePasswordRequest {
    #[garde(custom(super::super::validate::sha256_hex))]
    #[schema(pattern = "^[0-9a-f]{64}$")]
    pub(crate) current_password_sha256: String,
    #[garde(custom(super::super::validate::sha256_hex))]
    #[schema(pattern = "^[0-9a-f]{64}$")]
    pub(crate) password_sha256: String,
}

#[derive(Serialize, ToSchema)]
pub(crate) struct TotpEnrollment {
    pub(crate) secret_base32: String,
    pub(crate) uri: String,
}

#[macros_derive::validated_schema]
#[derive(Deserialize, ToSchema, Validate)]
pub(crate) struct EnableTotpRequest {
    #[garde(custom(super::super::validate::sha256_hex))]
    #[schema(pattern = "^[0-9a-f]{64}$")]
    pub(crate) current_password_sha256: String,
    #[garde(custom(super::super::validate::totp_secret))]
    #[schema(pattern = "^[A-Z2-7]{32}$")]
    pub(crate) secret_base32: String,
    #[garde(custom(super::super::validate::totp_code))]
    #[schema(pattern = "^[0-9]{6}$")]
    pub(crate) code: String,
}

#[macros_derive::validated_schema]
#[derive(Deserialize, ToSchema, Validate)]
pub(crate) struct DisableTotpRequest {
    #[garde(custom(super::super::validate::sha256_hex))]
    #[schema(pattern = "^[0-9a-f]{64}$")]
    pub(crate) current_password_sha256: String,
    #[garde(custom(super::super::validate::totp_code))]
    #[schema(pattern = "^[0-9]{6}$")]
    pub(crate) code: String,
}

#[macros_derive::validated_schema(patch)]
#[derive(Default, Deserialize, ToSchema, Validate)]
pub(crate) struct UpdatePreferencesRequest {
    #[garde(skip)]
    pub(crate) language: Option<Option<Locale>>,
    #[garde(skip)]
    pub(crate) theme: Option<crate::theme::Theme>,
    #[garde(skip)]
    pub(crate) alert_email: Option<bool>,
    #[garde(skip)]
    pub(crate) alerts: Option<BTreeSet<super::super::typ::Im>>,
}

#[macros_derive::validated_schema]
#[derive(Debug, Deserialize, ToSchema, Validate)]
pub(crate) struct SetImRequest {
    #[garde(skip)]
    pub(crate) platform: super::super::typ::Im,
    /// Use null to unbind.
    #[garde(inner(custom(super::super::validate::im_id(self.platform))))]
    #[serde(deserialize_with = "Option::deserialize")]
    #[schema(required = true)]
    pub(crate) id: Option<String>,
}
