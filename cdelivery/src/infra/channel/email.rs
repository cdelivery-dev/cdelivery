mod authres;

use std::sync::Arc;

use cipher::MasterKey;
use futures_util::TryStreamExt;
use jiff::Timestamp;
use rustls_platform_verifier::ConfigVerifierExt;

use crate::concept;
use crate::config;
use crate::error::{ErrorKind, Result};

type Session = async_imap::Session<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>;
type Smtp = mail_send::SmtpClient<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>;

pub(crate) struct Channel {
    cfg: concept::setting::Email,
}

struct Inbound {
    text: String,
    timestamp: Timestamp,
}

impl Channel {
    pub(crate) fn new(cfg: concept::setting::Email) -> Self {
        Self { cfg }
    }

    pub(crate) async fn send(&self, to: &str, subject: &str, text: &str) -> Result<()> {
        super::send(async {
            let mut client = self.smtp_session().await?;
            let message = mail_send::mail_builder::MessageBuilder::new()
                .from(self.cfg.address().as_str())
                .to(to)
                .subject(subject)
                .text_body(text);
            client.send(message).await?;
            Ok(())
        })
        .await
    }

    pub(crate) async fn verify(&self) -> Result<()> {
        let mut client = self.smtp_session().await?;
        client.noop().await?;
        client.quit().await?;
        if let Some(imap) = self.cfg.imap() {
            let mut session = self.imap_session(imap).await?;
            select_inbox(&mut session).await?;
            session.logout().await?;
        }
        Ok(())
    }

    pub(crate) async fn poll(
        &self,
        docker: &super::super::docker::Docker,
        key: &MasterKey,
        cursor: &mut Option<(u32, u32)>,
    ) -> Result<()> {
        let Some(imap) = self.cfg.imap() else {
            return Ok(());
        };
        let mut session = self.imap_session(imap).await?;
        let (validity, uid_next) = match select_inbox(&mut session).await {
            Ok(inbox) => inbox,
            Err(error) => {
                session.logout().await?;
                return Err(error);
            }
        };
        let last = match *cursor {
            Some((seen, uid)) if seen == validity => uid,
            _ => {
                *cursor = Some((validity, uid_next.saturating_sub(1)));
                session.logout().await?;
                return Ok(());
            }
        };
        let start = last.saturating_add(1);
        let found = session.uid_search(format!("UID {start}:*")).await?;
        for uid in new_uids(found, last) {
            let step = self.admit_uid(&mut session, docker, key, imap, uid);
            match tokio::time::timeout(config::AGENT_CHANNEL_TIMEOUT.unsigned_abs(), step).await {
                Ok(Ok(admitted)) => {
                    if let Some((user, inbound)) = admitted {
                        deliver(&user, inbound, source_id(self.cfg.address(), validity, uid));
                    }
                    *cursor = Some((validity, uid));
                }
                Ok(Err(e)) => return Err(e),
                Err(_) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        format!("email UID {uid} processing timed out"),
                    )
                    .into());
                }
            }
        }
        session.logout().await?;
        Ok(())
    }

    async fn smtp_session(&self) -> Result<Smtp> {
        let smtp = self.cfg.smtp();
        Ok(
            mail_send::SmtpClientBuilder::new(smtp.host().as_str(), smtp.port())
                .map_err(std::io::Error::other)?
                .timeout(config::AGENT_CHANNEL_TIMEOUT.unsigned_abs())
                .implicit_tls(smtp.port() == 465)
                .credentials((self.cfg.address().as_str(), smtp.password().as_str()))
                .connect()
                .await?,
        )
    }

    async fn imap_session(&self, imap: &concept::setting::Imap) -> Result<Session> {
        let tcp = tokio::net::TcpStream::connect((imap.host().as_str(), imap.port())).await?;
        let tls_config =
            rustls::ClientConfig::with_platform_verifier().map_err(std::io::Error::other)?;
        let server_name = rustls_pki_types::ServerName::try_from(imap.host().clone())
            .map_err(|_| ErrorKind::InvalidBody(format!("invalid tls host in {}", imap.host())))?;
        let tls = tokio_rustls::TlsConnector::from(Arc::new(tls_config))
            .connect(server_name, tcp)
            .await?;
        let mut client = async_imap::Client::new(tls);
        if client.read_response().await?.is_none() {
            return Err(ErrorKind::Imap(async_imap::error::Error::ConnectionLost).into());
        }
        Ok(client
            .login(self.cfg.address(), imap.password())
            .await
            .map_err(|(e, _)| e)?)
    }

    async fn admit_uid(
        &self,
        session: &mut Session,
        docker: &super::super::docker::Docker,
        key: &MasterKey,
        imap: &concept::setting::Imap,
        uid: u32,
    ) -> Result<Option<(concept::user::User, Inbound)>> {
        let fetches: Vec<_> = session
            .uid_fetch(uid.to_string(), "(RFC822 INTERNALDATE)")
            .await?
            .try_collect()
            .await?;
        let Some((fetch, bytes)) = fetches
            .iter()
            .find_map(|fetch| fetch.body().map(|body| (fetch, body)))
        else {
            tracing::debug!("email uid {uid} fetch returned no body, skipped");
            return Ok(None);
        };
        let Some(message) = mail_parser::MessageParser::default().parse(bytes) else {
            tracing::debug!("email uid {uid} is unparseable, skipped");
            return Ok(None);
        };
        let timestamp = fetch
            .internal_date()
            .and_then(|date| Timestamp::from_second(date.timestamp()).ok())
            .or_else(|| {
                message
                    .date()
                    .filter(mail_parser::DateTime::is_valid)
                    .and_then(|date| Timestamp::from_second(date.to_timestamp()).ok())
            });
        let Some(timestamp) = timestamp else {
            tracing::debug!("email uid {uid} has no valid source timestamp, skipped");
            return Ok(None);
        };
        let from = message
            .from()
            .and_then(|from| from.first())
            .and_then(|mailbox| mailbox.address());
        let user = match from {
            Some(address) => super::super::user::find_by_email(docker, key, address)
                .await?
                .map(concept::versioned::Versioned::into_inner),
            None => None,
        };
        let header = message
            .headers()
            .all("Authentication-Results")
            .find_map(|header| header.value().as_text());
        let passed = authres::trusted_pass(header, imap.authserv_id());
        let (Some(user), true) = (user, passed) else {
            tracing::debug!("email uid {uid} rejected by gate, dropped");
            return Ok(None);
        };
        Ok(Some((user, inbound_of(&message, timestamp))))
    }
}

fn deliver(user: &concept::user::User, inbound: Inbound, id: uuid::Uuid) {
    let timeline = concept::message::Timeline::Email { user: user.id() };
    super::super::message::MESSAGES.emit(&concept::message::Message::with_id(
        id,
        timeline,
        insight_wire::message::Role::User,
        user.name(),
        inbound.text,
        inbound.timestamp,
    ));
}

fn source_id(address: &str, validity: u32, uid: u32) -> uuid::Uuid {
    let source = format!("email:{address}:{validity}:{uid}");
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, source.as_bytes())
}

fn inbound_of(message: &mail_parser::Message<'_>, timestamp: Timestamp) -> Inbound {
    Inbound {
        text: super::super::event::truncate_bytes(
            message.body_text(0).unwrap_or_default().as_ref(),
            config::AGENT_INBOUND_TEXT_MAX,
        )
        .to_owned(),
        timestamp,
    }
}

fn new_uids(found: impl IntoIterator<Item = u32>, last: u32) -> Vec<u32> {
    let mut uids: Vec<u32> = found.into_iter().filter(|&uid| uid > last).collect();
    uids.sort_unstable();
    uids.dedup();
    uids
}

async fn select_inbox(session: &mut Session) -> Result<(u32, u32)> {
    let mailbox = session.select("INBOX").await?;
    let (Some(validity), Some(uid_next)) = (mailbox.uid_validity, mailbox.uid_next) else {
        return Err(ErrorKind::Imap(async_imap::error::Error::Bad(
            "SELECT INBOX returned no UIDVALIDITY or UIDNEXT".to_owned(),
        ))
        .into());
    };
    Ok((validity, uid_next))
}
