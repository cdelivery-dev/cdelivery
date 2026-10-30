use crate::config;

pub(crate) enum Lookup {
    Found(String),
    Absent,
    Unavailable,
}

pub(crate) struct Challenge {
    http: reqwest::Client,
    base_url: String,
    inflight: tokio::sync::Semaphore,
}

impl Challenge {
    pub(crate) fn new(http: reqwest::Client, endpoint: &str) -> Self {
        Self {
            http,
            base_url: format!("http://{endpoint}{}", pingress_wire::CHALLENGE_PATH_PREFIX),
            inflight: tokio::sync::Semaphore::new(config::CHALLENGE_INFLIGHT_CAP),
        }
    }

    pub(crate) async fn lookup(&self, token: &str) -> Lookup {
        if !well_formed(token) {
            return Lookup::Absent;
        }
        let Ok(_permit) = self.inflight.try_acquire() else {
            tracing::debug!("challenge lookup shed: in-flight cap reached");
            return Lookup::Unavailable;
        };
        let url = format!("{}{token}", self.base_url);
        match self.http.get(&url).send().await {
            Ok(resp) if resp.status() == reqwest::StatusCode::OK => {
                read_key_authorization(resp, token).await
            }
            Ok(resp) if resp.status() == reqwest::StatusCode::NOT_FOUND => Lookup::Absent,
            Ok(resp) => {
                tracing::warn!("challenge lookup returned {}", resp.status());
                Lookup::Unavailable
            }
            Err(error) => {
                tracing::warn!(error = ?error, "challenge lookup failed");
                Lookup::Unavailable
            }
        }
    }
}

async fn read_key_authorization(resp: reqwest::Response, token: &str) -> Lookup {
    if resp
        .content_length()
        .is_none_or(|len| len > config::CHALLENGE_BODY_CAP)
    {
        tracing::warn!("challenge body missing or exceeds length cap");
        return Lookup::Unavailable;
    }
    match resp.text().await {
        Ok(body) if key_authorization_shaped(&body, token) => Lookup::Found(body),
        Ok(_) => {
            tracing::warn!("challenge body violates key authorization shape");
            Lookup::Unavailable
        }
        Err(error) => {
            tracing::warn!(error = ?error, "challenge body read failed");
            Lookup::Unavailable
        }
    }
}

fn key_authorization_shaped(body: &str, token: &str) -> bool {
    body.strip_prefix(token)
        .and_then(|rest| rest.strip_prefix('.'))
        .is_some_and(|thumb| {
            !thumb.is_empty()
                && thumb
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
}

fn well_formed(token: &str) -> bool {
    (22..=128).contains(&token.len())
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
