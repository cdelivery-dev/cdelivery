mod setup;

use std::io::Write as _;
use std::process::ExitCode;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

use crate::error::{ErrorKind, Result};
use crate::{concept, http, infra, mcp, operation, usecase};

#[expect(clippy::print_stdout)]
pub(crate) async fn run(args: &[String]) -> Result<ExitCode> {
    let (email, admin_names) = match args {
        [command] if command == "setup" => return setup::run().await,
        [command, kind] if command == "contract" => {
            println!("{}", contract(kind)?);
            return Ok(ExitCode::SUCCESS);
        }
        [command, args @ ..] if command == "create-api-key" => {
            return create_api_key(args).await.map(|()| ExitCode::SUCCESS);
        }
        [command, email] if command == "rescue" => (email, None),
        [command, email, first_name, last_name] if command == "add-admin" => {
            (email, Some((first_name, last_name)))
        }
        _ => usage("Invalid command or arguments."),
    };
    let key = infra::keyring::load()?.ok_or(ErrorKind::NotInitialized)?;
    let docker = infra::docker::Docker::connect(docker::DEFAULT_SOCKET_PATH).await?;
    let password = temporary()?;
    let password_sha256 = sha256_hex(&password);
    match admin_names {
        None => usecase::user::rescue(&docker, &key, email, &password_sha256).await?,
        Some((first_name, last_name)) => {
            usecase::user::create_managed(
                &docker,
                &key,
                first_name.clone(),
                last_name.clone(),
                email.clone(),
                password_sha256,
                concept::user::Role::Admin,
            )
            .await?;
        }
    }
    println!("Temporary password for {email}: {password}");
    Ok(ExitCode::SUCCESS)
}

async fn create_api_key(args: &[String]) -> Result<()> {
    let [email, name, role, options @ ..] = args else {
        usage("API key creation requires an email, name, and role.");
    };
    if !(1..=64).contains(&name.len()) {
        usage("API key name must contain 1–64 UTF-8 bytes.");
    }
    let role = match role.as_str() {
        "Reader" => concept::user::Role::Reader,
        "Operator" => concept::user::Role::Operator,
        "Admin" => concept::user::Role::Admin,
        _ => usage("API key role must be Reader, Operator, or Admin."),
    };
    let expires_at = match options {
        [] => None,
        [option, value] if option == "--expires-at" => Some(
            value
                .parse::<jiff::Timestamp>()
                .unwrap_or_else(|error| usage(&format!("Invalid API key expiry: {error}"))),
        ),
        _ => usage("Expected only an optional --expires-at <timestamp>."),
    };
    let now = jiff::Timestamp::now();
    if expires_at.is_some_and(|expiry| expiry <= now) {
        usage("API key expiry must be in the future.");
    }
    let key = infra::keyring::load()?.ok_or(ErrorKind::NotInitialized)?;
    let docker = infra::docker::Docker::connect(docker::DEFAULT_SOCKET_PATH).await?;
    let user = usecase::user::get_by_email(&docker, &key, email).await?;
    if role > user.role() {
        return Err(ErrorKind::Forbidden.into());
    }
    let (token, _) = usecase::user::issue_api_key(
        &docker,
        &key,
        user.id(),
        name.clone(),
        expires_at,
        role,
        now,
    )
    .await?;
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{token}")?;
    stdout.flush()?;
    Ok(())
}

fn contract(kind: &str) -> Result<String> {
    let value = match kind {
        "openapi" => serde_json::to_value(http::openapi()?)?,
        "mcp" => operation::catalog::SPECS
            .iter()
            .zip(mcp::tool::all())
            .map(|(spec, tool)| {
                serde_json::json!({
                    "requirement": format!("{:?}", spec.requirement),
                    "tool": tool,
                })
            })
            .collect(),
        _ => usage("Contract format must be openapi or mcp."),
    };
    Ok(serde_json::to_string_pretty(&value)?)
}

#[expect(clippy::exit, clippy::print_stderr)]
fn usage(message: &str) -> ! {
    eprintln!(
        "{message}\n\nUsage:\n  cdelivery setup\n  cdelivery rescue <email>\n  cdelivery add-admin <email> <first_name> <last_name>\n  cdelivery create-api-key <email> <name> <Reader|Operator|Admin> [--expires-at <timestamp>]\n  cdelivery contract <openapi|mcp>\n\nSetup reads JSON from stdin: first_name, last_name, email, password. Run in a separate container using the deployed image and host networking. Initialized or Resumed means startup was submitted, not ready; Resumed preserves the previous account. AlreadyInitialized changes nothing. Failure may occur after changes were saved.\n\nAPI key names: 1–64 UTF-8 bytes. Expiry: future RFC 3339 timestamp with Z or UTC offset; omitted means no expiry. Each invocation prints only its new key."
    );
    std::process::exit(2)
}

fn temporary() -> Result<String> {
    let mut bytes = [0u8; 15];
    getrandom::fill(&mut bytes)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text)
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .filter_map(|nibble| char::from_digit(nibble.into(), 16))
        .collect()
}
