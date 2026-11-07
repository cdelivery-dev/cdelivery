macro_rules! cases {
    (@restore) => { async |_| Ok(()) };
    (@restore $restore:path) => { $restore };
    ($($(#[$attribute:meta])* $name:ident: $label:literal => $case:path $(, $argument:expr)* $(=> $restore:path)?;)*) => {
        $(
            $(#[$attribute])*
            #[tokio::test]
            async fn $name() -> crate::support::Result<()> {
                crate::support::run($label, async |context| $case(context $(, $argument)*).await, cases!(@restore $($restore)?)).await
            }
        )*
    };
}

cases! {
    setup_and_host_recovery: "setup" => crate::auth::setup;
    authentication_and_permissions: "auth" => crate::auth::permissions;
    rest_and_mcp_contracts: "contracts" => crate::contracts::protocols;
    catalog_permissions_before_decoding: "catalog-permissions" => crate::contracts::permissions;
    account_boundaries_and_recovery: "accounts" => crate::accounts::lifecycle;
    credential_expiry_and_origin: "credentials" => crate::accounts::credentials;
    totp_enrollment_replay_and_cooldown: "totp" => crate::accounts::totp;
    application_boundaries_and_cleanup: "app-boundaries" => crate::boundaries::apps;
    local_settings_and_bindings: "settings" => crate::boundaries::settings;
    duty_notification_fallback: "duty" => crate::duty::notifications, &["disabled", "ok", "runtime", "read", "model-error", "tool-error", "empty", "noop", "receipt", "cancel", "timeout", "channel-error"] => crate::duty::restore;
    duty_permission_and_write_receipts: "duty-permissions" => crate::duty::notifications, &["demote", "receipt-cancel"] => crate::duty::restore;
    #[ignore = "waits for the real 15-minute patrol interval"]
    duty_patrol_finding: "duty-patrol-finding" => crate::duty::patrol, "patrol" => crate::duty::restore;
    #[ignore = "waits for the real 15-minute patrol interval"]
    duty_patrol_silent: "duty-patrol-silent" => crate::duty::patrol, "noop" => crate::duty::restore;
    runtime_current_tasks_and_recovery: "runtime" => crate::duty::runtime;
    invitation_revocation_and_node_labels: "invitations" => crate::boundaries::invitations;
    route_precedence_and_invalidation: "routing" => crate::routing::lifecycle;
    guardian_operator_decisions: "guardian" => crate::guardian::decisions;
    insight_query_contracts: "queries" => crate::telemetry::queries;
    #[ignore = "requires cluster"]
    insight_idle_connection_recovery: "idle-connection" => crate::telemetry::idle_connection;
    platform_components_disable_and_restore: "platform" => crate::platform::lifecycle => crate::platform::restore;
    pingress_enable_acme_and_recovery: "pingress-acme" => crate::pingress::enable => crate::pingress::restore;
    dynamic_replica_bounds_and_fixed_transition: "scaling" => crate::platform::scaling;
    upgrade_cdelivery: "upgrade-cdelivery" => crate::upgrade::component, "cdelivery" => crate::upgrade::restore;
    upgrade_volume_agent: "upgrade-volume" => crate::upgrade::component, "cdelivery-agent" => crate::upgrade::restore;
    upgrade_pingress: "upgrade-pingress" => crate::upgrade::component, "pingress" => crate::upgrade::restore;
    upgrade_insight_server: "upgrade-insight" => crate::upgrade::component, "insight" => crate::upgrade::restore;
    upgrade_insight_agent: "upgrade-collector" => crate::upgrade::component, "insight-agent" => crate::upgrade::restore;
    upgrade_hurl: "upgrade-hurl" => crate::upgrade::hurl => crate::upgrade::restore;
    upgrade_release_rejections: "upgrade-catalog" => crate::upgrade::catalog => crate::upgrade::restore;
    app_lifecycle_and_artifacts: "apps" => crate::apps::lifecycle;
    secrets_and_runtime_injection: "secrets" => crate::secrets::lifecycle;
    tini_shared_configs_and_deployment_pins: "tini" => crate::tini::deployments;
    image_access_preflight: "image-preflight" => crate::images::preflight;
    xhealth_guardian_and_jobs: "jobs" => crate::apps::jobs;
    xhealth_manual_runs_and_recovery: "xhealth" => crate::xhealth::run_now;
    xjob_intervals_timeouts_and_recovery: "xjobs" => crate::xjobs::lifecycle;
    volumes_survive_restart: "volumes" => crate::apps::volumes;
    https_manual_certificates: "https-manual" => crate::https::certificates;
    https_acme_and_renewal: "https-acme" => crate::https::acme;
    insight_telemetry: "telemetry" => crate::telemetry::telemetry;
    proxy_access_records: "access" => crate::telemetry::access;
    #[ignore = "requires cluster"]
    node_failure_and_recovery: "recovery" => crate::nodes::recovery => crate::nodes::restore;
    node_membership_and_join_invitations: "membership" => crate::nodes::membership;
}
