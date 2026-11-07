mod fixture;

use serde_json::{Value, json};

use crate::support::{self, Context, Result, ensure, eventually, send, string};

pub(crate) async fn preflight(context: &Context) -> Result<()> {
    let fixture = fixture::Fixture::start(context).await?;
    let result = verify(context, &fixture).await;
    std::fs::write(
        context.root.join("image-auth-calls.json"),
        serde_json::to_vec_pretty(&fixture.calls().await)?,
    )?;
    fixture.stop().await?;
    result
}

#[expect(clippy::too_many_lines)]
async fn verify(context: &Context, fixture: &fixture::Fixture) -> Result<()> {
    let name = support::name("image");
    let path = format!("/api/apps/{name}");
    let image = fixture.image("exists", false);
    let credentials = json!({"username":"reader","token":fixture::PASSWORD});
    let upload = json!({"target":"/preflight.txt","data_base64":"YQ=="});
    let create = json!({"name":name,"group":"tests","image":image,"image_auth":credentials,"replicas":{"Fixed":1},"container_options":{"command":["sleep","86400"]},"xconfigs":[upload]});
    for (image, auth) in [
        (image.clone(), Value::Null),
        (image.clone(), json!({"username":"reader","token":"wrong"})),
        (
            image.clone(),
            json!({"username":"denied","token":fixture::PASSWORD}),
        ),
        (fixture.image("missing", false), credentials.clone()),
    ] {
        let mut rejected_create = create.clone();
        rejected_create["image"] = json!(image);
        rejected_create["image_auth"] = auth;
        rejected(context.request("POST", "/api/apps")?.json(&rejected_create)).await?;
        send(context.request("GET", &path)?, 404).await?;
        ensure(
            configs(context, &name)?.is_empty(),
            "failed creation must not upload configs",
        )?;
    }
    context.api("POST", "/api/apps", Some(create)).await?;
    running(context, &name, &image).await?;
    ensure(
        fixture
            .calls()
            .await
            .iter()
            .any(|(user, scope)| user == "reader" && scope.ends_with(":pull")),
        "Docker must request pull scope with the supplied user",
    )?;
    context
        .api(
            "PATCH",
            &path,
            Some(json!({"image":fixture.image("next",false),"image_auth":{"username":"reader"}})),
        )
        .await?;
    running(context, &name, &fixture.image("next", false)).await?;

    fixture.mode(fixture::Mode::Unavailable).await;
    let before = context.inspect_service(&name)?;
    for patch in [
        json!({}),
        json!({"image":fixture.image("next",false),"image_auth":credentials}),
    ] {
        context.api("PATCH", &path, Some(patch)).await?;
    }
    let after = context.inspect_service(&name)?;
    ensure(
        before[0]["Version"] == after[0]["Version"] && before[0]["Spec"] == after[0]["Spec"],
        "unchanged image and auth must preserve the stored version",
    )?;
    context
        .api(
            "PATCH",
            &path,
            Some(json!({"description":"no registry request"})),
        )
        .await?;
    ensure(
        fixture.requests().await == 0,
        "unrelated updates must skip the registry",
    )?;
    rejected(
        context
            .request("PATCH", &path)?
            .json(&json!({"image":image})),
    )
    .await?;

    fixture.mode(fixture::Mode::Private).await;
    rejected(
        context
            .request("PATCH", &path)?
            .json(&json!({"image":fixture.image("exists",true)})),
    )
    .await?;
    let required: Value = send(
        context.request("PATCH", &path)?.json(
            &json!({"image":fixture.image("exists",true),"image_auth":{"username":"reader"}}),
        ),
        422,
    )
    .await?
    .json()
    .await?;
    ensure(
        required["code"] == "SecretRequired",
        "a new registry must require a new token",
    )?;
    for other in [true, false] {
        context
            .api(
                "PATCH",
                &path,
                Some(json!({"image":fixture.image("exists",other),"image_auth":credentials})),
            )
            .await?;
        running(context, &name, &fixture.image("exists", other)).await?;
    }

    context.api("PUT", &format!("{path}/xhealth"), Some(json!({"method":{"Custom":{"command":["true"]}},"interval":{"unit":"Hours","value":24},"target":{"InternalPort":{"port":8080}},"guardian":true}))).await?;
    eventually("live health passes", || async {
        ensure(
            context.api("GET", &format!("{path}/xhealth"), None).await?["status"]
                .get("Passed")
                .is_some(),
            "live app must be healthy",
        )
    })
    .await?;
    let before = context.inspect_service(&name)?;
    let before_configs = configs(context, &name)?;
    for mut patch in [
        json!({"image":fixture.image("missing",false)}),
        json!({"image_auth":{"username":"reader","token":"wrong"}}),
        json!({"image_auth":{"username":"denied","token":fixture::PASSWORD}}),
        json!({"image_auth":null}),
    ] {
        patch["xconfigs"] = json!([{"Upload":upload}]);
        rejected(context.request("PATCH", &path)?.json(&patch)).await?;
        let after = context.inspect_service(&name)?;
        ensure(
            before[0]["Version"] == after[0]["Version"] && before[0]["Spec"] == after[0]["Spec"],
            "rejected update must preserve the live service",
        )?;
        ensure(
            configs(context, &name)? == before_configs,
            "rejected update must not upload configs",
        )?;
        send(context.request("GET", &format!("{path}/guardian"))?, 404).await?;
    }
    let mcp = crate::contracts::mcp(context, &context.credentials.api_key, "tools/call", json!({"name":"app_update","arguments":{"name":name,"changes":{"image":fixture.image("missing",false)}}})).await?;
    let problem: Value = serde_json::from_str(string(&mcp["result"]["content"][0], "text")?)?;
    ensure(
        mcp["result"]["isError"] == true
            && problem["code"] == "ImageCheckFailed"
            && problem["status"] == 422,
        "MCP must expose the same image failure",
    )?;

    let browser = context
        .login(&context.credentials.email, &context.credentials.password)
        .await?;
    let page = send(
        browser
            .post(format!("{}/apps/{name}/spec", context.environment.base_url))
            .header("Origin", &context.environment.base_url)
            .header("Accept-Language", "zh-CN")
            .form(&[(
                "payload",
                json!({"changes":{"image":fixture.image("missing",false)}}).to_string(),
            )]),
        422,
    )
    .await?
    .text()
    .await?;
    ensure(
        page.contains("ImageCheckFailed")
            && page.contains("无法验证")
            && page.contains("manifest")
            && !page.contains(fixture::PASSWORD),
        "UI must show localized image failure and original Docker diagnostic",
    )?;

    let current = context.api("GET", &path, None).await?;
    send(context.request("PATCH", &path)?.json(&json!({"image":fixture.image("next",false),"xconfigs":[{"InUse":current["xconfigs"][0]},{"Upload":{"target":"/second.txt","data_base64":"Yg=="}}]})),202).await?;
    eventually("checked candidate promotes with configs", || async {
        let live = context.api("GET", &path, None).await?;
        ensure(
            live["image"] == fixture.image("next", false)
                && live["xconfigs"]
                    .as_array()
                    .is_some_and(|configs| configs.len() == 2),
            "promotion must preserve both configs",
        )?;
        send(context.request("GET", &format!("{path}/guardian"))?, 404).await?;
        Ok(())
    })
    .await?;
    context
        .api("DELETE", &format!("{path}/xhealth"), None)
        .await?;
    running(context, &name, &fixture.image("next", false)).await?;

    let job_path = format!("{path}/xjobs/auth");
    let job_service = format!("cdelivery-xjob-{name}_auth");
    let job = context.api("PUT", &job_path, Some(json!({"interval":{"unit":"Hours","value":24},"timeout":{"unit":"Minutes","value":2},"command":["true"]}))).await?;
    context
        .api("POST", &format!("{job_path}/run"), None)
        .await?;
    completed(context, &job_path, &job["last_run"]).await?;
    let job_template = context.inspect_service(&job_service)?[0]["Spec"]["TaskTemplate"].clone();

    fixture.mode(fixture::Mode::Slow).await;
    let started = std::time::Instant::now();
    rejected(
        context
            .request("PATCH", &path)?
            .json(&json!({"image":image})),
    )
    .await?;
    ensure(
        (9..20).contains(&started.elapsed().as_secs()),
        "image checks must use the existing ten-second Docker timeout",
    )?;

    fixture.mode(fixture::Mode::Anonymous).await;
    context
        .api(
            "PATCH",
            &path,
            Some(json!({"image":fixture.image("public",false),"image_auth":null,"xconfigs":[]})),
        )
        .await?;
    running(context, &name, &fixture.image("public", false)).await?;
    let app = context.api("GET", &path, None).await?;
    ensure(
        app["image_auth"].is_null() && app["xconfigs"] == json!([]),
        "explicit clears must persist",
    )?;
    let calls = fixture.calls().await;
    ensure(
        !calls.is_empty() && calls.iter().all(|(user, _)| user == "anonymous"),
        "preflight and deployment must not inherit cleared credentials",
    )?;

    context
        .api(
            "PATCH",
            &job_path,
            Some(json!({"timeout":{"unit":"Minutes","value":3}})),
        )
        .await?;
    let current = context.inspect_service(&job_service)?;
    ensure(
        current[0]["Spec"]["TaskTemplate"] == job_template,
        "updating job configuration must preserve the previous task template",
    )?;
    fixture.mode(fixture::Mode::Private).await;
    let previous = context.api("GET", &job_path, None).await?;
    let mut spec = current[0]["Spec"].clone();
    let force_update = spec["TaskTemplate"]["ForceUpdate"]
        .as_u64()
        .ok_or_else(|| support::ErrorKind::Check("missing job force-update counter".into()))?;
    spec["TaskTemplate"]["ForceUpdate"] = json!(force_update.wrapping_add(1));
    context.engine(
        "POST",
        &format!(
            "/services/{job_service}/update?version={}",
            current[0]["Version"]["Index"]
        ),
        &spec,
    )?;
    let retained = completed(context, &job_path, &previous["last_run"]).await?;
    let calls = fixture.calls().await;
    std::fs::write(
        context.root.join("job-auth-retained.json"),
        json!(calls).to_string(),
    )?;
    ensure(
        !calls.is_empty() && calls.iter().all(|(user, _)| user == "reader"),
        &format!(
            "updating job configuration must retain the old image's authentication: {calls:?}"
        ),
    )?;

    fixture.mode(fixture::Mode::Anonymous).await;
    context
        .api("POST", &format!("{job_path}/run"), None)
        .await?;
    completed(context, &job_path, &retained["last_run"]).await?;
    ensure(
        context.inspect_service(&job_service)?[0]["Spec"]["TaskTemplate"]["ContainerSpec"]["Image"]
            == fixture.image("public", false),
        "a new job run must adopt the app's current image",
    )?;
    let calls = fixture.calls().await;
    ensure(
        !calls.is_empty() && calls.iter().all(|(user, _)| user == "anonymous"),
        "a new job run must adopt the app's cleared credentials",
    )?;
    context.api("DELETE", &path, None).await?;
    Ok(())
}

fn configs(context: &Context, name: &str) -> Result<String> {
    context.docker(&[
        "config",
        "ls",
        "--filter",
        &format!("label=dev.cdelivery.app_name={name}"),
        "--format",
        "{{.ID}}",
    ])
}

async fn running(context: &Context, name: &str, image: &str) -> Result<()> {
    eventually("requested image runs", || async {
        let tasks = context.docker(&[
            "service",
            "ps",
            "--no-trunc",
            "--filter",
            "desired-state=running",
            "--format",
            "{{.Image}} {{.CurrentState}}",
            name,
        ])?;
        ensure(
            tasks
                .lines()
                .any(|line| line.starts_with(image) && line.contains("Running")),
            &format!("requested image must actually run: {tasks}"),
        )
    })
    .await
}

async fn rejected(request: reqwest::RequestBuilder) -> Result<()> {
    let problem: Value = send(request, 422).await?.json().await?;
    let detail = string(&problem, "detail")?;
    ensure(
        problem["code"] == "ImageCheckFailed"
            && detail.contains("could not verify access to image")
            && detail.contains(": ")
            && !detail.contains(fixture::PASSWORD),
        &format!("image failure must retain the Docker diagnostic without credentials: {problem}"),
    )
}

async fn completed(context: &Context, path: &str, previous_run: &Value) -> Result<Value> {
    eventually("job completes a new run", || async {
        let job = context.api("GET", path, None).await?;
        ensure(
            job["status"].get("Passed").is_some() && job["last_run"] != *previous_run,
            &format!("job must complete a new run: {job}"),
        )?;
        Ok(job)
    })
    .await
}
