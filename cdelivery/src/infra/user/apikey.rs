use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use cipher::{Domain, MasterKey};
use uuid::Uuid;

const PREFIX: &str = "cdk_";

pub(crate) fn mint(key: &MasterKey, user_id: Uuid) -> (String, String) {
    let raw = [user_id.into_bytes(), Uuid::new_v4().into_bytes()].concat();
    let token = format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(raw));
    let idx = index(key, &token);
    (token, idx)
}

fn index(key: &MasterKey, token: &str) -> String {
    key.blind_index(Domain::ApiKeyIndex, token.as_bytes())
}

pub(crate) fn parse(key: &MasterKey, token: &str) -> Option<(Uuid, String)> {
    let raw = URL_SAFE_NO_PAD.decode(token.strip_prefix(PREFIX)?).ok()?;
    let (owner, random) = raw.split_first_chunk::<16>()?;
    (random.len() == 16).then(|| (Uuid::from_bytes(*owner), index(key, token)))
}
