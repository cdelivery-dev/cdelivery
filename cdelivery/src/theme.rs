use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
pub(crate) enum Theme {
    #[default]
    Auto,
    Light,
    Dark,
}

impl Theme {
    pub(crate) fn token(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

tokio::task_local! {
    pub(crate) static THEME: Theme;
}

pub(crate) fn current() -> Theme {
    THEME.try_with(|theme| *theme).unwrap_or_default()
}
