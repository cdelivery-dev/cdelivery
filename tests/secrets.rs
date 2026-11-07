use base64::Engine as _;
use serde_json::{Value, json};

use crate::support::{self, Context, Result, ensure, eventually, send, string};

pub(crate) async fn metadata(context: &Context, name: &str) -> Result<Value> {
    context.api("GET", "/api/secrets", None).await?["secrets"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["name"] == name))
        .cloned()
        .ok_or_else(|| support::ErrorKind::Check(format!("missing Secret {name}")).into())
}

#[expect(clippy::too_many_lines)]
pub(crate) async fn lifecycle(context: &Context) -> Result<()> {
    let name = support::name("sec");
    let path = format!("/api/apps/{name}");
    let text_name = format!("{name}-text");
    let file_name = format!("{name}-file");
    let text_path = format!("/api/secrets/{text_name}");
    let file_path = format!("/api/secrets/{file_name}");
    let plaintext = "  multiline-test-secret\n第二行\n";
    let bytes = [0, 1, 255, b'\n'];
    let text64 = base64::engine::general_purpose::STANDARD.encode(plaintext);
    let file64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    let key = context
        .api(
            "POST",
            "/api/apikeys",
            Some(json!({"name":name,"role":"Operator"})),
        )
        .await?;
    let token = string(&key, "key")?;
    let reader = context
        .api(
            "POST",
            "/api/apikeys",
            Some(json!({"name":format!("{name}-reader"),"role":"Reader"})),
        )
        .await?;
    let reader_token = string(&reader, "key")?;
    send(
        context
            .client
            .get(format!("{}/api/secrets", context.environment.base_url))
            .bearer_auth(reader_token),
        200,
    )
    .await?;
    send(
        context
            .client
            .post(format!("{}/api/secrets", context.environment.base_url))
            .bearer_auth(reader_token)
            .json(&json!({"name":text_name,"content":{"Text":"denied"}})),
        403,
    )
    .await?;
    send(
        context
            .client
            .delete(format!("{}{text_path}", context.environment.base_url))
            .bearer_auth(reader_token),
        403,
    )
    .await?;
    context
        .api(
            "DELETE",
            &format!("/api/apikeys/{}", string(&reader, "id")?),
            None,
        )
        .await?;
    for body in [
        json!({"name":text_name,"description":"x".repeat(65),"content":{"Text":plaintext}}),
        json!({"name":text_name,"content":{"Text":"bad\u{0}value"}}),
    ] {
        send(context.request("POST", "/api/secrets")?.json(&body), 422).await?;
    }
    for (content, status) in [
        (json!({"Text":"界".repeat(171_000)}), 422),
        (
            json!({"File":base64::engine::general_purpose::STANDARD.encode(vec![1; 512_000])}),
            413,
        ),
    ] {
        send(
            context
                .request("POST", "/api/secrets")?
                .json(&json!({"name":text_name,"content":content})),
            status,
        )
        .await?;
    }
    let created: Value = send(
        context.client
            .post(format!("{}/api/secrets", context.environment.base_url))
            .bearer_auth(token)
            .json(
                &json!({"name":text_name,"description":"字".repeat(64),"content":{"Text":plaintext}}),
            ),
        201,
    )
    .await?
    .json()
    .await?;
    ensure(
        created == json!({"ok":true}),
        "creation returns only success",
    )?;
    context
        .api(
            "POST",
            "/api/secrets",
            Some(json!({"name":file_name,"content":{"File":file64}})),
        )
        .await?;
    let text = metadata(context, &text_name).await?;
    let file = metadata(context, &file_name).await?;
    for resource in [&text_path, &file_path] {
        send(context.request("GET", resource)?, 405).await?;
    }
    let text_ref = json!({"name":text_name,"id":text["id"]});
    let file_ref = json!({"name":file_name,"id":file["id"]});
    for resource in ["/api/secrets", "/api/secrets?kind=File"] {
        let metadata = context.api("GET", resource, None).await?;
        ensure(
            !metadata.to_string().contains("multiline-test-secret"),
            "reads must not return Secret content",
        )?;
        ensure(
            metadata.get("content").is_none(),
            "Secret content is write-only",
        )?;
    }
    let filtered = context.api("GET", "/api/secrets?kind=Text", None).await?;
    ensure(
        filtered["secrets"]
            .as_array()
            .is_some_and(|items| items.iter().all(|item| item["kind"] == "Text")),
        "kind filtering",
    )?;
    send(
        context
            .request("POST", "/api/secrets")?
            .json(&json!({"name":text_name,"content":{"Text":"replace"}})),
        409,
    )
    .await?;
    let command = vec![
        "sh",
        "-c",
        "mkdir -p /www; printf '%s' \"$TOKEN\" | base64 | tr -d '\\n' > /www/token; base64 /fixture.bin | tr -d '\\n' > /www/file; printf healthy > /www/index.html; exec httpd -f -p 8080 -h /www",
    ];
    let options = json!({"command":command,"env":[{"name":"TOKEN","value":text_ref},{"name":"NORMAL","value":"ordinary"}]});
    let mut create = json!({"name":name,"group":"tests","image":context.environment.images["fixture"].reference,
        "replicas":{"Fixed":1},"container_options":options,"xconfigs":[{"target":"/fixture.bin","secret":file_ref}]});
    create["container_options"]["env"][0]["value"] = file_ref.clone();
    send(context.request("POST", "/api/apps")?.json(&create), 422).await?;
    create["container_options"] = options.clone();
    create["xconfigs"][0]["secret"] = text_ref.clone();
    send(context.request("POST", "/api/apps")?.json(&create), 422).await?;
    create["xconfigs"][0]["secret"] = file_ref.clone();
    context.api("POST", "/api/apps", Some(create)).await?;
    context.ready(&name, 1).await?;
    let (machine, container) = eventually("Secret app container", || async {
        for machine in &context.environment.machines {
            let id = context.machine(
                machine,
                &[
                    "ps",
                    "--filter",
                    &format!("label=com.docker.swarm.service.name={name}"),
                    "--format",
                    "{{.ID}}",
                ],
            )?;
            if !id.trim().is_empty() {
                return Ok((machine.clone(), id.trim().to_owned()));
            }
        }
        Err(support::ErrorKind::Check("container not running".into()).into())
    })
    .await?;
    for (file, encoded) in [("token", &text64), ("file", &file64)] {
        let actual = context.machine(
            &machine,
            &[
                "exec",
                &container,
                "wget",
                "-qO-",
                &format!("http://localhost:8080/{file}"),
            ],
        )?;
        ensure(
            actual.trim() == encoded,
            "startup injection must preserve every byte",
        )?;
    }
    let direct = context.machine(
        &machine,
        &["exec", &container, "sh", "-c", "test -z \"${TOKEN+x}\""],
    )?;
    ensure(
        direct.is_empty(),
        "ordinary exec must not inherit injected env",
    )?;
    let before = context.engine("GET", &format!("/services/{name}"), &Value::Null)?;
    let peer = format!("{name}-peer");
    context.api("POST", "/api/apps", Some(json!({"name":peer,"group":"tests","image":context.environment.images["fixture"].reference,"replicas":{"Fixed":1},"container_options":{"command":["sleep","3600"]},"xconfigs":[{"target":"/fixture.bin","secret":file_ref}]}))).await?;
    for secret in [&text, &file] {
        let label = format!("dev.cdelivery.secret_ref.{}", string(secret, "id")?);
        ensure(
            before["Spec"]["Labels"][&label] == "true",
            "Secret ID label",
        )?;
        let matches = context.docker(&[
            "service",
            "ls",
            "--filter",
            &format!("label={label}=true"),
            "--format",
            "{{.Name}}",
        ])?;
        let mut names: Vec<_> = matches.lines().collect();
        names.sort_unstable();
        let expected = if secret == &text {
            vec![name.as_str()]
        } else {
            vec![name.as_str(), peer.as_str()]
        };
        ensure(names == expected, "Secret filter excludes unrelated apps")?;
    }
    context
        .api("DELETE", &format!("/api/apps/{peer}"), None)
        .await?;
    ensure(
        before["Spec"]["Labels"]
            .get("dev.cdelivery.uses_secret")
            .is_none(),
        "no legacy label",
    )?;
    ensure(
        !before.to_string().contains("multiline-test-secret"),
        "service spec must contain references only",
    )?;
    ensure(
        before["Spec"]["TaskTemplate"]["ContainerSpec"]["Init"] == true,
        "Text enables init",
    )?;
    let saved = context.api("GET", &path, None).await?;
    ensure(
        saved["xconfigs"][0]["secret"] == file_ref,
        "file binding round trip",
    )?;
    ensure(
        saved["container_options"]["env"][1]["value"] == text_ref,
        "env binding round trip",
    )?;
    context
        .api("PATCH", &path, Some(json!({"container_options":options})))
        .await?;
    let unchanged = context.engine("GET", &format!("/services/{name}"), &Value::Null)?;
    ensure(
        unchanged["Version"] == before["Version"],
        "repeated binding must be a no-op",
    )?;
    send(context.request("DELETE", &text_path)?, 409).await?;
    send(context.request("DELETE", &file_path)?, 409).await?;
    for invalid in [
        json!({"xconfigs":[{"Secret":{"target":"/run/cdelivery/env/TOKEN","secret":file_ref}}]}),
        json!({"container_options":{"env":[{"name":"TOKEN","value":text_ref},{"name":"TOKEN","value":"duplicate"}]}}),
        json!({"container_options":{"env":[{"name":"CD_INIT_ARCH","value":text_ref}]}}),
        json!({"container_options":{"env":[{"name":"TOKEN","value":{"name":text_name,"id":"stale"}}]}}),
    ] {
        send(context.request("PATCH", &path)?.json(&invalid), 422).await?;
    }
    let probe = format!(
        "test \"$(printf '%s' \"$TOKEN\" | base64 | tr -d '\\n')\" = '{text64}' && test \"$(base64 /fixture.bin | tr -d '\\n')\" = '{file64}'"
    );
    let job_path = format!("{path}/xjobs/secret-check");
    context.api("PUT", &job_path, Some(json!({"interval":{"unit":"Hours","value":24},"timeout":{"unit":"Minutes","value":1},"command":["sh","-c",probe]}))).await?;
    context
        .api("POST", &format!("{job_path}/run"), None)
        .await?;
    eventually("job Secret injection", || async {
        ensure(
            context.api("GET", &job_path, None).await?["status"]
                .get("Passed")
                .is_some(),
            "job must pass",
        )
    })
    .await?;
    let health_path = format!("{path}/xhealth");
    context.api("PUT", &health_path, Some(json!({"method":{"Custom":{"command":["sh","-c",probe]}},"interval":{"unit":"Minutes","value":1},"target":{"InternalPort":{"port":8080}},"guardian":false}))).await?;
    eventually("health Secret injection", || async {
        ensure(
            context.api("GET", &health_path, None).await?["status"]
                .get("Passed")
                .is_some(),
            "health must pass",
        )
    })
    .await?;
    let mut hurl_options = options.clone();
    hurl_options["env"]
        .as_array_mut()
        .ok_or_else(|| support::ErrorKind::Check("env fixture".into()))?
        .push(json!({"name":"HURL_VARIABLE_EXPECTED","value":text_ref}));
    context
        .api(
            "PATCH",
            &path,
            Some(json!({"container_options":hurl_options})),
        )
        .await?;
    context.ready(&name, 1).await?;
    context.api("PATCH", &health_path, Some(json!({"method":{"Hurl":{"file":"GET {{TARGET_URL}}/token\nHTTP 200\n[Asserts]\nbody base64Decode utf8Decode == \"{{EXPECTED}}\"\n","version":context.environment.images["hurl"].version}},"guardian":true}))).await?;
    eventually("Hurl Secret variables", || async {
        ensure(
            context.api("GET", &health_path, None).await?["status"]
                .get("Passed")
                .is_some(),
            "Hurl must pass",
        )
    })
    .await?;
    hurl_options["env"]
        .as_array_mut()
        .ok_or_else(|| support::ErrorKind::Check("env fixture".into()))?
        .push(json!({"name":"ADDED","value":"candidate"}));
    send(
        context
            .request("PATCH", &path)?
            .json(&json!({"container_options":hurl_options})),
        202,
    )
    .await?;
    eventually("Guardian Secret promotion", || async {
        let app = context.api("GET", &path, None).await?;
        ensure(
            app["container_options"]["env"]
                .as_array()
                .is_some_and(|env| env.iter().any(|entry| entry["name"] == "ADDED")),
            "candidate must be promoted",
        )
    })
    .await?;
    context.api("DELETE", &health_path, None).await?;
    let promoted = context.engine("GET", &format!("/services/{name}"), &Value::Null)?;
    ensure(
        promoted["Spec"]["Labels"]
            .as_object()
            .is_some_and(|labels| {
                labels
                    .keys()
                    .filter(|key| key.starts_with("dev.cdelivery.secret_ref."))
                    .count()
                    == 2
            }),
        "Guardian preserves deduplicated Secret labels",
    )?;
    context.api("DELETE", &job_path, None).await?;
    context
        .api(
            "PATCH",
            &path,
            Some(json!({"container_options":{"command":command},"xconfigs":[]})),
        )
        .await?;
    let removed = context.engine("GET", &format!("/services/{name}"), &Value::Null)?;
    for secret in [&text, &file] {
        let label = format!("dev.cdelivery.secret_ref.{}", string(secret, "id")?);
        ensure(
            removed["Spec"]["Labels"][&label] == "true",
            "rollback references remain indexed",
        )?;
    }
    ensure(
        removed["Spec"]["TaskTemplate"]["ContainerSpec"]["Init"] != true
            && removed["Spec"]["TaskTemplate"]["ContainerSpec"]["Configs"]
                .as_array()
                .is_none_or(Vec::is_empty),
        "last Text removal removes custom init",
    )?;
    send(context.request("DELETE", &text_path)?, 409).await?;
    send(context.request("DELETE", &file_path)?, 409).await?;
    context
        .api("POST", &format!("{path}/rollback"), None)
        .await?;
    context.ready(&name, 1).await?;
    ensure(
        context.engine("GET", &format!("/services/{name}"), &Value::Null)?["Spec"]["TaskTemplate"]
            ["ContainerSpec"]["Configs"]
            == before["Spec"]["TaskTemplate"]["ContainerSpec"]["Configs"],
        "rollback restores pinned init",
    )?;
    context.api("DELETE", &path, None).await?;
    let stateful = support::name("sfile");
    let volume = format!("{}{stateful}", cdelivery_wire::VOLUME_PREFIX);
    let node = context.docker(&["node", "inspect", "self", "--format", "{{.ID}}"])?;
    let node = node.trim();
    let machine = &context.environment.machines[0];
    context.machine(machine, &["volume", "create", &volume])?;
    let command = format!(
        "test \"$(base64 /fixture.bin | tr -d '\\n')\" = '{file64}' && touch /data/checked && exec sleep 3600"
    );
    context.api("POST", "/api/apps", Some(json!({"name":stateful,"group":"tests","image":context.environment.images["fixture"].reference,"replicas":{"Fixed":1},"mounts":{"node_id":node,"mounts":[{"volume":stateful,"target":"/data"}]},"xconfigs":[{"target":"/fixture.bin","secret":file_ref}],"container_options":{"command":["sh","-c",command]}}))).await?;
    context.ready(&stateful, 1).await?;
    ensure(
        context.engine("GET", &format!("/services/{stateful}"), &Value::Null)?["Spec"]["TaskTemplate"]
            ["ContainerSpec"]["Init"]
            != true,
        "File-only stateful apps do not enable custom init",
    )?;
    send(context.request("DELETE", &file_path)?, 409).await?;
    context
        .api("DELETE", &format!("/api/apps/{stateful}"), None)
        .await?;
    eventually("stateful Secret volume cleanup", || async {
        ensure(
            context
                .machine(
                    machine,
                    &[
                        "ps",
                        "--all",
                        "--filter",
                        &format!("volume={volume}"),
                        "--format",
                        "{{.ID}}",
                    ],
                )?
                .trim()
                .is_empty(),
            "volume containers are still being removed",
        )
    })
    .await?;
    context.machine(machine, &["volume", "rm", &volume])?;
    let abandoned = support::name("prev");
    let abandoned_path = format!("/api/apps/{abandoned}");
    context.app(&abandoned, "healthy").await?;
    ensure(
        context.engine("GET", &format!("/services/{abandoned}"), &Value::Null)?["Spec"]["TaskTemplate"]
            ["ContainerSpec"]["Init"]
            != true,
        "ordinary apps do not enable custom init",
    )?;
    context
        .api(
            "PATCH",
            &abandoned_path,
            Some(json!({"xconfigs":[{"Secret":{"target":"/fixture.bin","secret":file_ref}}]})),
        )
        .await?;
    context.ready(&abandoned, 1).await?;
    context
        .api("POST", &format!("{abandoned_path}/rollback"), None)
        .await?;
    context.ready(&abandoned, 1).await?;
    context.api("DELETE", &file_path, None).await?;
    context.api("DELETE", &abandoned_path, None).await?;
    context
        .api(
            "POST",
            "/api/secrets",
            Some(json!({"name":file_name,"content":{"File":file64}})),
        )
        .await?;
    let stale = json!({"name":abandoned,"group":"tests","image":context.environment.images["fixture"].reference,"replicas":{"Fixed":1},"xconfigs":[{"target":"/fixture.bin","secret":file_ref}]});
    send(context.request("POST", "/api/apps")?.json(&stale), 422).await?;
    eventually("Secret cleanup", || async {
        context.api("DELETE", &text_path, None).await?;
        context.api("DELETE", &file_path, None).await?;
        Ok(())
    })
    .await?;
    send(
        context
            .client
            .delete(format!("{}{text_path}", context.environment.base_url))
            .bearer_auth(token),
        204,
    )
    .await?;
    context
        .api(
            "DELETE",
            &format!("/api/apikeys/{}", string(&key, "id")?),
            None,
        )
        .await?;
    Ok(())
}
