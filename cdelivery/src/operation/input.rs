pub(crate) trait Compose<Parts> {
    fn compose(parts: Parts) -> Self;
}

impl<T> Compose<(T,)> for T {
    fn compose((value,): (T,)) -> Self {
        value
    }
}

macro_rules! composite_input {
    ($name:ident { $($(#[$attr:meta])* $field:ident: $ty:ty),+ $(,)? }) => {
        #[macros_derive::validated_schema]
        #[derive(serde::Deserialize, utoipa::ToSchema, garde::Validate)]
        pub(crate) struct $name {
            $($(#[$attr])* pub(crate) $field: $ty,)+
        }

        impl $crate::operation::input::Compose<($($ty,)+)> for $name {
            fn compose(($($field,)+): ($($ty,)+)) -> Self {
                Self { $($field),+ }
            }
        }
    };
}

pub(crate) use composite_input;

macro_rules! string_input {
    ($name:ident { $($field:ident: $validator:ident),+ }) => {
        composite_input!($name {
            $(
                #[garde(custom(super::validate::$validator))]
                $field: String
            ),+
        });
    };
}

#[macros_derive::validated_schema]
#[derive(serde::Deserialize, utoipa::ToSchema, garde::Validate)]
#[expect(clippy::empty_structs_with_brackets)]
pub(crate) struct Empty {}

impl Compose<()> for Empty {
    fn compose((): ()) -> Self {
        Self {}
    }
}

string_input!(Name { name: valid_name });
string_input!(Id { id: valid_node_id });
string_input!(App { app: valid_name });
string_input!(AppJob {
    app: valid_name,
    job: valid_name
});
string_input!(AppConfig {
    app: valid_name,
    config: valid_xconfig_name
});
string_input!(Domain {
    domain: valid_domain
});
composite_input!(ApiKeyId {
    #[garde(skip)]
    id: uuid::Uuid
});
composite_input!(UserId {
    #[garde(skip)]
    id: uuid::Uuid
});
