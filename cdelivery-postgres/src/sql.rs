use crate::error::{ErrorKind, Result};

pub(crate) fn execute(dsn: &str, sql: &str, wait: bool) -> Result<()> {
    if dsn.contains('\0') || sql.contains('\0') || sql.trim().is_empty() {
        return Err(ErrorKind::Invalid(
            "DSN and SQL must be nonempty and contain no NUL".to_owned(),
        )
        .into());
    }
    let started = jiff::Timestamp::now();
    let connection = loop {
        match libpq::Connection::start(dsn) {
            Ok(connection) => {
                loop {
                    match connection.poll() {
                        libpq::poll::Status::Ok | libpq::poll::Status::Failed => break,
                        _ => {}
                    }
                    if jiff::Timestamp::now().duration_since(started).as_secs() >= 120 {
                        return Err(ErrorKind::Invalid(
                            "PostgreSQL connection timed out".to_owned(),
                        )
                        .into());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                if connection.status() == libpq::connection::Status::Ok {
                    break connection;
                }
                if !wait || jiff::Timestamp::now().duration_since(started).as_secs() >= 120 {
                    return Err(ErrorKind::Invalid(
                        connection
                            .error_message()
                            .unwrap_or("PostgreSQL connection failed")
                            .to_owned(),
                    )
                    .into());
                }
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
            Err(error)
                if !wait || jiff::Timestamp::now().duration_since(started).as_secs() >= 120 =>
            {
                return Err(error.into());
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_secs(2)),
        }
    };
    let result = connection.exec(sql);
    match result.status() {
        libpq::Status::CommandOk | libpq::Status::TuplesOk => {}
        libpq::Status::CopyIn | libpq::Status::CopyOut | libpq::Status::CopyBoth => {
            return Err(ErrorKind::Invalid("COPY streaming is not supported".to_owned()).into());
        }
        _ => {
            return Err(ErrorKind::Invalid(
                result
                    .error_message()?
                    .unwrap_or_else(|| "PostgreSQL command failed".to_owned()),
            )
            .into());
        }
    }
    if connection.transaction_status() != libpq::transaction::Status::Idle {
        return Err(ErrorKind::Invalid(
            "SQL left an unfinished transaction; connection was closed without commit".to_owned(),
        )
        .into());
    }
    Ok(())
}
