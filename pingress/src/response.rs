use pingora::Result;
use pingora::http::ResponseHeader;
use pingora::protocols::http::ServerSession;
use pingora::proxy::Session;

pub(crate) fn set_server(response: &mut ResponseHeader) -> Result<()> {
    response.insert_header(http::header::SERVER, "Pingress")
}

pub(crate) async fn text(session: &mut Session, body: Option<bytes::Bytes>) -> Result<()> {
    let mut response = ResponseHeader::build(http::StatusCode::OK, Some(2))?;
    response.insert_header(http::header::CONTENT_TYPE, "text/plain")?;
    set_server(&mut response)?;
    session
        .write_response_header(Box::new(response), false)
        .await?;
    session.write_response_body(body, true).await
}

pub(crate) async fn error(session: &mut ServerSession, code: u16) -> Result<()> {
    let mut response = ServerSession::generate_error(code);
    set_server(&mut response)?;
    session
        .write_error_response(response, bytes::Bytes::new())
        .await
}
