#[macro_export]
macro_rules! located_error {
    ($err:ident => $kind:ident, from { $($ty:ty => $variant:ident),* $(,)? }) => {
        #[derive(Debug)]
        pub struct $err {
            kind: $kind,
            location: &'static ::std::panic::Location<'static>,
        }

        pub type Result<T> = ::std::result::Result<T, $err>;

        impl $err {
            #[track_caller]
            pub(crate) fn new(kind: $kind) -> Self {
                Self { kind, location: ::std::panic::Location::caller() }
            }
        }

        impl ::std::fmt::Display for $err {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                ::std::fmt::Display::fmt(&self.kind, f)
            }
        }

        impl ::std::error::Error for $err {
            fn source(&self) -> ::std::option::Option<&(dyn ::std::error::Error + 'static)> {
                ::std::error::Error::source(&self.kind)
            }
        }

        impl ::std::convert::From<$kind> for $err {
            #[track_caller]
            fn from(kind: $kind) -> Self {
                Self::new(kind)
            }
        }

        $(
            impl ::std::convert::From<$ty> for $err {
                #[track_caller]
                fn from(e: $ty) -> Self {
                    Self::new($kind::$variant(e))
                }
            }
        )*
    };
}

#[macro_export]
macro_rules! mirror_enum {
    ($(#[$meta:meta])* $name:ident from $($src:ident)::+ { $($body:tt)* }) => {
        $crate::mirror_enum! { @decl $(#[$meta])* $name { $($body)* } }
        $crate::mirror_enum! { @impl $($src)::+ => $name { $($body)* } }
    };
    ($(#[$meta:meta])* $name:ident into $($dst:ident)::+ { $($body:tt)* }) => {
        $crate::mirror_enum! { @decl $(#[$meta])* $name { $($body)* } }
        $crate::mirror_enum! { @impl $name => $($dst)::+ { $($body)* } }
    };
    ($(#[$meta:meta])* $name:ident both $($peer:ident)::+ { $($body:tt)* }) => {
        $crate::mirror_enum! { @decl $(#[$meta])* $name { $($body)* } }
        $crate::mirror_enum! { @impl $($peer)::+ => $name { $($body)* } }
        $crate::mirror_enum! { @impl $name => $($peer)::+ { $($body)* } }
    };
    ($($from:ident)::+ => $($to:ident)::+ { $($body:tt)* }) => {
        $crate::mirror_enum! { @impl $($from)::+ => $($to)::+ { $($body)* } }
    };
    ($($a:ident)::+ <-> $($b:ident)::+ { $($body:tt)* }) => {
        $crate::mirror_enum! { @impl $($a)::+ => $($b)::+ { $($body)* } }
        $crate::mirror_enum! { @impl $($b)::+ => $($a)::+ { $($body)* } }
    };
    (@decl $(#[$meta:meta])* $name:ident {
        $($(#[$vm:meta])* $v:ident $({ $($(#[$fm:meta])* $f:ident: $t:ty),+ $(,)? })?),+ $(,)?
    }) => {
        $(#[$meta])*
        pub(crate) enum $name {
            $($(#[$vm])* $v $({ $($(#[$fm])* $f: $t),+ })?),+
        }
    };
    (@impl $($from:ident)::+ => $($to:ident)::+ {
        $($(#[$vm:meta])* $v:ident $({ $($(#[$fm:meta])* $f:ident: $t:ty),+ $(,)? })?),+ $(,)?
    }) => {
        impl ::std::convert::From<$($from)::+> for $($to)::+ {
            fn from(value: $($from)::+) -> Self {
                type Src = $($from)::+;
                match value {
                    $(Src::$v $({ $($f),+ })? => Self::$v $({ $($f),+ })?),+
                }
            }
        }
    };
    (@decl $(#[$meta:meta])* $name:ident {
        $($(#[$vm:meta])* $v:ident $(($(#[$fm:meta])* $f:ident: $t:ty))?),+ $(,)?
    }) => {
        $(#[$meta])*
        pub(crate) enum $name {
            $($(#[$vm])* $v $(($(#[$fm])* $t))?),+
        }
    };
    (@impl $($from:ident)::+ => $($to:ident)::+ {
        $($(#[$vm:meta])* $v:ident $(($(#[$fm:meta])* $f:ident: $t:ty))?),+ $(,)?
    }) => {
        impl ::std::convert::From<$($from)::+> for $($to)::+ {
            fn from(value: $($from)::+) -> Self {
                type Src = $($from)::+;
                match value {
                    $(Src::$v $(($f))? => Self::$v $(($f))?),+
                }
            }
        }
    };
}
