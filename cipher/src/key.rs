use chacha20poly1305::aead::Generate;
use serde::{Deserialize, Serialize};
use serde_with::{
    base64::{Base64, UrlSafe},
    serde_as,
};
use zeroize::{Zeroize, Zeroizing};

use crate::error::Result;
use crate::text::Cipher;

#[derive(Debug)]
pub enum Domain {
    Pingress,
    User,
    UserEmailIndex,
    UserImIndex,
    Session,
    ApiKeyIndex,
    AppImageAuth,
    Guardian,
    Acme,
    Insight,
    Setting,
    Volume,
}

impl Domain {
    const fn context(self) -> &'static str {
        match self {
            Self::Pingress => "CDelivery 2025-01-13 pingress xchacha20poly1305 v1",
            Self::User => "CDelivery 2025-07-07 user record xchacha20poly1305 v1",
            Self::UserEmailIndex => "CDelivery 2025-07-07 user email-index blake3-mac v1",
            Self::UserImIndex => "CDelivery 2026-08-01 user im-index blake3-mac v1",
            Self::Session => "CDelivery 2026-08-03 session xchacha20poly1305 v1",
            Self::ApiKeyIndex => "CDelivery 2025-07-08 api-key-index blake3-mac v1",
            Self::AppImageAuth => "CDelivery 2025-07-10 app image-auth xchacha20poly1305 v1",
            Self::Guardian => "CDelivery 2026-07-28 guardian xchacha20poly1305 v1",
            Self::Acme => "CDelivery 2025-07-13 acme store xchacha20poly1305 v1",
            Self::Insight => "CDelivery 2026-07-15 insight xchacha20poly1305 v1",
            Self::Setting => "CDelivery 2026-07-22 setting xchacha20poly1305 v1",
            Self::Volume => "CDelivery 2026-09-12 volume command xchacha20poly1305 v1",
        }
    }
}

#[serde_as]
#[derive(Serialize, Deserialize)]
pub struct MasterKey(#[serde_as(as = "Base64<UrlSafe>")] [u8; 32]);

impl std::fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("MasterKey").field(&"<redacted>").finish()
    }
}

impl Drop for MasterKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl MasterKey {
    pub fn generate() -> Result<Self> {
        Ok(Self(<[u8; 32]>::try_generate()?))
    }

    fn derive(&self, context: &str) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(blake3::derive_key(context, &self.0))
    }

    pub fn subkey(&self, domain: Domain) -> Zeroizing<[u8; 32]> {
        self.derive(domain.context())
    }

    pub fn cipher_for(&self, domain: Domain) -> Cipher {
        Cipher::from_key(&self.subkey(domain))
    }

    pub fn blind_index(&self, domain: Domain, input: &[u8]) -> String {
        blake3::keyed_hash(&self.subkey(domain), input)
            .to_hex()
            .to_string()
    }
}
