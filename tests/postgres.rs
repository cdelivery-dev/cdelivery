use std::process::Command;

use cdelivery_wire::postgres::{self, Command as PostgresCommand, Response, Target};

#[path = "process.rs"]
mod process;

#[derive(Debug, thiserror::Error)]
enum ErrorKind {
    #[error(transparent)]
    Io(std::io::Error),
    #[error(transparent)]
    Http(reqwest::Error),
    #[error(transparent)]
    Cipher(cipher::Error),
    #[error(transparent)]
    Fields(postgres::Error),
    #[error(transparent)]
    Process(process::Error),
    #[error("{0}")]
    Check(String),
}

macros::located_error! { Error => ErrorKind, from {
    std::io::Error => Io, reqwest::Error => Http, cipher::Error => Cipher,
    postgres::Error => Fields, process::Error => Process,
}}

fn docker(args: &[&str]) -> Result<String> {
    Ok(process::run(
        Command::new("docker").args(args),
        b"",
        jiff::SignedDuration::from_secs(120).unsigned_abs(),
        None,
    )?)
}

async fn request(
    client: &reqwest::Client,
    url: &str,
    cipher: &cipher::Cipher,
    command: PostgresCommand,
) -> Result<Response> {
    let body = cipher.seal(&command)?;
    let response = client
        .post(url)
        .body(body)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(cipher.open(&response)?)
}

async fn execute(
    client: &reqwest::Client,
    url: &str,
    cipher: &cipher::Cipher,
    target: Target,
    sql: &str,
    success: bool,
) -> Result<()> {
    let response = request(
        client,
        url,
        cipher,
        PostgresCommand::Execute {
            dependency: "database".to_owned(),
            target,
            sql: cipher::Secret::new(sql.to_owned()),
        },
    )
    .await?;
    if matches!(
        (&response, success),
        (Response::Done, true) | (Response::Failed { .. }, false)
    ) {
        return Ok(());
    }
    Err(ErrorKind::Check(format!("unexpected SQL execution result: {response:?}")).into())
}

async fn ready(client: &reqwest::Client, url: &str) -> Result<()> {
    let started = jiff::Timestamp::now();
    loop {
        if client
            .get(format!("{url}health"))
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
        {
            return Ok(());
        }
        if jiff::Timestamp::now().duration_since(started).as_secs() >= 60 {
            return Err(ErrorKind::Check("executor did not become ready".to_owned()).into());
        }
        tokio::time::sleep(jiff::SignedDuration::from_millis(200).unsigned_abs()).await;
    }
}

#[tokio::test]
#[expect(clippy::too_many_lines)]
async fn local_executor() -> Result<()> {
    let id = uuid::Uuid::new_v4().to_string();
    let name = format!("cdelivery-postgres-probe-{id}");
    let label = format!("dev.cdelivery.postgres-probe={id}");
    let source = format!("{name}-source");
    let preview = format!("{name}-preview");
    let worker = format!("{name}-worker");
    let directory = std::env::temp_dir().join(&name);
    let key = [71u8; 32];
    let cipher = cipher::Cipher::from_key(&key);
    let client = reqwest::Client::builder()
        .timeout(jiff::SignedDuration::from_secs(150).unsigned_abs())
        .build()?;
    let image = match std::env::var("CDELIVERY_POSTGRES_IMAGE") {
        Ok(image) => image,
        Err(std::env::VarError::NotPresent) => {
            let package = process::run(
                Command::new("cargo").args([
                    "pkgid",
                    "--locked",
                    "--package",
                    "cdelivery-postgres",
                ]),
                b"",
                jiff::SignedDuration::from_secs(30).unsigned_abs(),
                None,
            )?;
            let (_, version) = package.trim().rsplit_once('#').ok_or_else(|| {
                ErrorKind::Check("executor package version is missing".to_owned())
            })?;
            format!("ghcr.io/cdelivery-dev/cdelivery-postgres:{version}")
        }
        Err(error) => return Err(ErrorKind::Check(error.to_string()).into()),
    };
    let result = async {
        docker(&["image", "inspect", &image])?;
        std::fs::create_dir(&directory)?;
        std::fs::write(directory.join("key"), key)?;
        let mut fields = postgres::Dsn::parse("postgresql://postgres:probe@source:5432/probe?sslmode=disable&application_name=integration%20probe")?.fields();
        fields.insert("provider".to_owned(), "Local".to_owned());
        fields.insert("app".to_owned(), "source".to_owned());
        std::fs::write(directory.join("database"), postgres::encode_fields(&fields)?)?;
        docker(&["network", "create", "--label", &label, &name])?;
        docker(&["run", "--detach", "--name", &source, "--label", &label, "--network", &name, "--network-alias", "source", "--env", "POSTGRES_PASSWORD=probe", "--env", "POSTGRES_DB=probe", "postgres:18-alpine"])?;
        docker(&["run", "--detach", "--name", &worker, "--label", &label, "--network", &name, "--publish", "127.0.0.1::5012", "--mount", &format!("type=bind,source={},target={},readonly", directory.join("key").display(), postgres::KEY_PATH), "--mount", &format!("type=bind,source={},target={}/database,readonly", directory.join("database").display(), postgres::SECRET_DIRECTORY), &image])?;
        let address = docker(&["port", &worker, "5012/tcp"])?;
        let url = format!("http://{}/", address.trim());
        ready(&client, &url).await?;
        let unauthorized = client.post(&url).body(cipher::Cipher::from_key(&[72u8; 32]).seal(&PostgresCommand::Execute {
            dependency: "database".to_owned(), target: Target::Production, sql: cipher::Secret::new("SELECT 1".to_owned()),
        })?).send().await?;
        if unauthorized.status() != reqwest::StatusCode::UNAUTHORIZED {
            return Err(ErrorKind::Check("executor accepted a command encrypted with another key".to_owned()).into());
        }
        let response = request(&client, &url, &cipher, PostgresCommand::Prepare {
            dependency: "database".to_owned(), name: "rehearsal".to_owned(), host: Some("preview".to_owned()), size: None, planetscale_sizes: None,
        }).await?;
        let Response::Prepared { dsn } = response else { return Err(ErrorKind::Check("expected prepared connection".to_owned()).into()); };
        let parsed = postgres::Dsn::parse(dsn.reveal())?;
        if parsed.host() != "preview" || parsed.port() != 5432 || !parsed.dsn().ends_with("?sslmode=disable&application_name=integration%20probe") {
            return Err(ErrorKind::Check("preview connection did not preserve the DSN contract".to_owned()).into());
        }
        let pending = execute(&client, &url, &cipher, Target::Preview, "CREATE TABLE batch (id integer PRIMARY KEY, value text); INSERT INTO batch VALUES(1, 'semi;colon'); SELECT * FROM batch;", true);
        let start = async {
            tokio::time::sleep(jiff::SignedDuration::from_secs(2).unsigned_abs()).await;
            docker(&["run", "--detach", "--name", &preview, "--label", &label, "--network", &name, "--network-alias", "preview", "--env", "POSTGRES_PASSWORD=probe", "--env", "POSTGRES_DB=probe", "postgres:18-alpine"])
        };
        let (executed, started) = tokio::join!(pending, start);
        started?;
        executed?;
        execute(&client, &url, &cipher, Target::Production, "DO $$ BEGIN IF to_regclass('batch') IS NOT NULL THEN RAISE EXCEPTION 'preview affected source'; END IF; END $$;", true).await?;
        for (sql, success) in [
            ("BEGIN; INSERT INTO batch VALUES(2, 'committed'); COMMIT;", true),
            ("BEGIN; INSERT INTO batch VALUES(3, 'uncommitted');", false),
            ("DO $$ BEGIN IF EXISTS(SELECT FROM batch WHERE id=3) THEN RAISE EXCEPTION 'unfinished transaction committed'; END IF; END $$;", true),
            ("INSERT INTO batch VALUES(4, 'before error'); SELECT 1/0; INSERT INTO batch VALUES(5, 'after error');", false),
            ("DO $$ BEGIN IF EXISTS(SELECT FROM batch WHERE id IN(4,5)) THEN RAISE EXCEPTION 'failed batch persisted'; END IF; END $$;", true),
            ("COPY batch TO STDOUT", false),
            ("COPY batch FROM STDIN", false),
            ("\\dt", false),
            ("SELECT 1\0;", false),
            ("DO $$ BEGIN IF (SELECT count(*) FROM batch) <> 2 THEN RAISE EXCEPTION 'unexpected data'; END IF; END $$;", true),
        ] {
            execute(&client, &url, &cipher, Target::Preview, sql, success).await?;
        }
        let response = request(&client, &url, &cipher, PostgresCommand::Cleanup { dependency: "database".to_owned(), name: "rehearsal".to_owned() }).await?;
        if !matches!(response, Response::Done) { return Err(ErrorKind::Check(format!("unexpected cleanup response: {response:?}")).into()); }
        execute(&client, &url, &cipher, Target::Preview, "SELECT 1", false).await?;
        let response = request(&client, &url, &cipher, PostgresCommand::Prepare { dependency: "database".to_owned(), name: "rehearsal".to_owned(), host: Some("preview".to_owned()), size: None, planetscale_sizes: None }).await?;
        if !matches!(response, Response::Prepared { .. }) { return Err(ErrorKind::Check(format!("unexpected repeated preparation response: {response:?}")).into()); }
        docker(&["restart", &worker])?;
        let address = docker(&["port", &worker, "5012/tcp"])?;
        let url = format!("http://{}/", address.trim());
        ready(&client, &url).await?;
        execute(&client, &url, &cipher, Target::Preview, "SELECT 1", false).await?;
        execute(&client, &url, &cipher, Target::Production, "SELECT 1; SELECT 2;", true).await?;
        Ok::<_, Error>(())
    }.await;
    let cleanup = (|| -> Result<()> {
        let containers = docker(&[
            "ps",
            "--all",
            "--quiet",
            "--filter",
            &format!("label={label}"),
        ])?;
        if !containers.trim().is_empty() {
            let mut args = vec!["rm", "--force", "--volumes"];
            args.extend(containers.split_whitespace());
            docker(&args)?;
        }
        let networks = docker(&[
            "network",
            "ls",
            "--quiet",
            "--filter",
            &format!("label={label}"),
        ])?;
        for network in networks.split_whitespace() {
            docker(&["network", "rm", network])?;
        }
        if directory.exists() {
            std::fs::remove_dir_all(&directory)?;
        }
        Ok(())
    })();
    result?;
    cleanup
}
