use std::future::{self, Future};
use std::marker::PhantomData;

use axum::extract::FromRequestParts;
use axum::response::IntoResponse;
use http::request::Parts;

use crate::boot::App;
use crate::error::{ErrorKind, Result};
use crate::{concept, operation};

pub(crate) struct Authorized<O: operation::catalog::Catalogued, R> {
    access: concept::permission::Access,
    marker: PhantomData<fn() -> (O, R)>,
}

impl<O: operation::catalog::Catalogued, R> Authorized<O, R> {
    pub(crate) fn access(&self) -> &concept::permission::Access {
        &self.access
    }

    pub(crate) async fn execute(&self, app: &App, input: O::Input) -> Result<O::Output> {
        operation::execute::<O>(app, &self.access, input, jiff::Timestamp::now()).await
    }

    pub(crate) async fn compose<P>(&self, app: &App, parts: P) -> Result<O::Output>
    where
        O::Input: operation::input::Compose<P>,
    {
        self.execute(
            app,
            <O::Input as operation::input::Compose<P>>::compose(parts),
        )
        .await
    }
}

impl<S, O, R> FromRequestParts<S> for Authorized<O, R>
where
    S: Send + Sync,
    O: operation::catalog::Catalogued,
    R: From<ErrorKind> + IntoResponse + Send,
{
    type Rejection = R;

    fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> impl Future<Output = std::result::Result<Self, R>> + Send {
        let result = match parts
            .extensions
            .get::<concept::permission::Access>()
            .cloned()
        {
            Some(access) if access.allows(O::REQUIREMENT) => Ok(Self {
                access,
                marker: PhantomData,
            }),
            Some(_) => Err(ErrorKind::Forbidden.into()),
            None => Err(ErrorKind::AuthRequired.into()),
        };
        future::ready(result)
    }
}
