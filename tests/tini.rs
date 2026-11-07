use serde_json::{Value, json};

use crate::support::{self, Context, Result, ensure, eventually, send, string};

#[expect(clippy::too_many_lines)]
pub(crate) async fn deployments(context: &Context) -> Result<()> {
    let name = support::name("tini");
    let peer = format!("{name}-peer");
    let path = format!("/api/apps/{name}");
    context
        .api("POST", "/api/groups", Some(json!({"name":peer})))
        .await?;
    context
        .api(
            "POST",
            "/api/secrets",
            Some(json!({"name":name,"content":{"Text":"fixture"}})),
        )
        .await?;
    let secret = crate::secrets::metadata(context, &name).await?;
    let options = json!({"command":["sleep","3600"],"env":[{"name":"TOKEN","value":{"name":name,"id":secret["id"]}}]});
    let create = json!({"name":name,"group":"tests","image":context.environment.images["fixture"].reference,"replicas":{"Fixed":1},"container_options":options});
    context
        .api("POST", "/api/apps", Some(create.clone()))
        .await?;
    context.ready(&name, 1).await?;
    let original = context.engine("GET", &format!("/services/{name}"), &Value::Null)?;
    let bundled = original["Spec"]["TaskTemplate"]["ContainerSpec"]["Configs"].clone();
    let configs = bundled
        .as_array()
        .ok_or_else(|| support::ErrorKind::Check("missing Tini configs".into()))?;
    ensure(
        configs.len() == 3,
        "one config per architecture and one selector",
    )?;
    context.api("DELETE", &path, None).await?;
    for config in configs {
        let config_name = string(config, "ConfigName")?;
        eventually("release bootstrap config", || async {
            context.engine("DELETE", &format!("/configs/{config_name}"), &Value::Null)?;
            Ok(())
        })
        .await?;
    }
    let selector = configs
        .iter()
        .find(|config| config["File"]["Name"] == "/sbin/docker-init")
        .ok_or_else(|| support::ErrorKind::Check("missing selector".into()))?;
    let selector_name = string(selector, "ConfigName")?;
    context.engine(
        "POST",
        "/configs/create",
        &json!({"Name":selector_name,"Data":"aW52YWxpZA==","Templating":{"Name":"golang"}}),
    )?;
    send(context.request("POST", "/api/apps")?.json(&create), 409).await?;
    let mut partial = Vec::new();
    for config in configs
        .iter()
        .filter(|config| config["File"]["Name"] != "/sbin/docker-init")
    {
        let native = context.engine(
            "GET",
            &format!("/configs/{}", string(config, "ConfigName")?),
            &Value::Null,
        )?;
        partial.push(native["ID"].clone());
    }
    send(context.request("GET", &path)?, 404).await?;
    context.engine("DELETE", &format!("/configs/{selector_name}"), &Value::Null)?;
    let mut peer_create = create.clone();
    peer_create["name"] = json!(peer);
    peer_create["group"] = json!(peer);
    let (first, second) = tokio::join!(
        context.api("POST", "/api/apps", Some(create)),
        context.api("POST", "/api/apps", Some(peer_create))
    );
    first?;
    second?;
    context.ready(&name, 1).await?;
    context.ready(&peer, 1).await?;
    let original = context.engine("GET", &format!("/services/{name}"), &Value::Null)?;
    let bundled = original["Spec"]["TaskTemplate"]["ContainerSpec"]["Configs"].clone();
    ensure(
        context.engine("GET", &format!("/services/{peer}"), &Value::Null)?["Spec"]["TaskTemplate"]
            ["ContainerSpec"]["Configs"]
            == bundled,
        "concurrent creates across groups share Secret and Config references",
    )?;
    ensure(
        bundled.as_array().is_some_and(|configs| {
            partial
                .iter()
                .all(|id| configs.iter().any(|config| &config["ConfigID"] == id))
        }),
        "retry reuses configs created before failure",
    )?;
    context
        .api("DELETE", &format!("/api/apps/{peer}"), None)
        .await?;
    context
        .api("DELETE", &format!("/api/groups/{peer}"), None)
        .await?;
    let mut created = Vec::new();
    for (index, prefix) in ["0.0.0", "999.0.0"].iter().enumerate() {
        let fixture_version = format!("{prefix}-{name}");
        let current = context.engine("GET", &format!("/services/{name}"), &Value::Null)?;
        let mut spec = current["Spec"].clone();
        let configs = spec["TaskTemplate"]["ContainerSpec"]["Configs"]
            .as_array_mut()
            .ok_or_else(|| support::ErrorKind::Check("missing Tini configs".into()))?;
        for config in configs {
            let old_name = string(config, "ConfigName")?.to_owned();
            let native = context.engine("GET", &format!("/configs/{old_name}"), &Value::Null)?;
            let suffix = old_name
                .rsplit('-')
                .next()
                .ok_or_else(|| support::ErrorKind::Check("config suffix".into()))?;
            let fixture_name = format!("cdelivery-tini-{fixture_version}-{suffix}");
            let mut definition = native["Spec"].clone();
            definition["Name"] = json!(fixture_name);
            let native = context.engine("POST", "/configs/create", &definition)?;
            config["ConfigID"] = native["ID"].clone();
            config["ConfigName"] = json!(fixture_name);
            created.push(fixture_name);
        }
        let pinned = spec["TaskTemplate"]["ContainerSpec"]["Configs"].clone();
        context.engine(
            "POST",
            &format!(
                "/services/{name}/update?version={}",
                current["Version"]["Index"]
            ),
            &spec,
        )?;
        context.ready(&name, if index == 0 { 1 } else { 2 }).await?;
        let before = context.engine("GET", &format!("/services/{name}"), &Value::Null)?;
        context.api("PATCH", &path, Some(json!({}))).await?;
        let unchanged = context.api("GET", &path, None).await?;
        context
            .api(
                "PATCH",
                &path,
                Some(json!({"container_options":unchanged["container_options"]})),
            )
            .await?;
        ensure(
            context.engine("GET", &format!("/services/{name}"), &Value::Null)?["Version"]
                == before["Version"],
            "no-op does not adopt a different platform bundle",
        )?;
        context
            .api(
                "PATCH",
                &path,
                Some(json!({"description":format!("metadata-{index}")})),
            )
            .await?;
        context
            .api("POST", &format!("{path}/restart"), None)
            .await?;
        context
            .api("PATCH", &path, Some(json!({"replicas":{"Fixed":2}})))
            .await?;
        ensure(
            context.engine("GET", &format!("/services/{name}"), &Value::Null)?["Spec"]["TaskTemplate"]
                ["ContainerSpec"]["Configs"]
                == pinned,
            "metadata, restart and scale keep the pin",
        )?;
        let mut updated_options = options.clone();
        updated_options["command"] = json!(["sleep", format!("{}", 3601 + index)]);
        context
            .api(
                "PATCH",
                &path,
                Some(json!({"container_options":updated_options})),
            )
            .await?;
        context.ready(&name, 2).await?;
        ensure(
            context.engine("GET", &format!("/services/{name}"), &Value::Null)?["Spec"]["TaskTemplate"]
                ["ContainerSpec"]["Configs"]
                == bundled,
            "deployment adopts bundled configs in both version directions",
        )?;
        context
            .api("POST", &format!("{path}/rollback"), None)
            .await?;
        context.ready(&name, 2).await?;
        ensure(
            context.engine("GET", &format!("/services/{name}"), &Value::Null)?["Spec"]["TaskTemplate"]
                ["ContainerSpec"]["Configs"]
                == pinned,
            "rollback restores previous configs",
        )?;
        if index == 0 {
            context.api("PUT", &format!("{path}/xhealth"), Some(json!({"method":{"Custom":{"command":["sh","-c","sleep 5; test \"$TOKEN\" = fixture"]}},"interval":{"unit":"Minutes","value":1},"target":{"InternalPort":{"port":8080}},"guardian":true}))).await?;
            send(
                context
                    .request("PATCH", &path)?
                    .json(&json!({"container_options":updated_options})),
                202,
            )
            .await?;
            let candidate = context.engine(
                "GET",
                &format!("/services/cdelivery-shadow-{name}"),
                &Value::Null,
            )?;
            ensure(
                candidate["Spec"]["TaskTemplate"]["ContainerSpec"]["Configs"] == bundled,
                "Guardian candidate fixes bundled references at creation",
            )?;
            eventually("Guardian pinned check", || async {
                let check_name = format!("cdelivery-verify-{name}");
                let services = context.docker(&[
                    "service",
                    "ls",
                    "--filter",
                    &format!("name={check_name}"),
                    "--format",
                    "{{.Name}}",
                ])?;
                ensure(
                    services.lines().any(|service| service == check_name),
                    "Guardian check is not created yet",
                )?;
                let check = context.engine(
                    "GET",
                    &format!("/services/cdelivery-verify-{name}"),
                    &Value::Null,
                )?;
                ensure(
                    check["Spec"]["TaskTemplate"]["ContainerSpec"]["Configs"] == bundled,
                    "Guardian check inherits candidate configs",
                )
            })
            .await?;
            eventually("Guardian promotion", || async {
                let current = context.engine("GET", &format!("/services/{name}"), &Value::Null)?;
                ensure(
                    current["Spec"]["TaskTemplate"]["ContainerSpec"]["Configs"] == bundled,
                    "Guardian publishes the fixed configs",
                )
            })
            .await?;
            context
                .api("DELETE", &format!("{path}/xhealth"), None)
                .await?;
        }
    }
    context.api("DELETE", &path, None).await?;
    eventually("Tini fixture cleanup", || async {
        context
            .api("DELETE", &format!("/api/secrets/{name}"), None)
            .await?;
        for config in &created {
            context.engine("DELETE", &format!("/configs/{config}"), &Value::Null)?;
        }
        Ok(())
    })
    .await?;
    Ok(())
}
