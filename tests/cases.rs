macro_rules! cases {
    ($($(#[$attribute:meta])* $name:ident: $label:literal => $case:path;)*) => {
        $(
            $(#[$attribute])*
            #[tokio::test]
            async fn $name() -> crate::support::Result<()> {
                crate::support::run($label, $case).await
            }
        )*
    };
}

cases! {
    setup_and_host_recovery: "setup" => crate::auth::setup;
    authentication_and_permissions: "auth" => crate::auth::permissions;
    rest_and_mcp_contracts: "contracts" => crate::contracts::protocols;
    app_lifecycle_and_artifacts: "apps" => crate::apps::lifecycle;
    xhealth_guardian_and_jobs: "jobs" => crate::apps::jobs;
    xhealth_manual_runs_and_recovery: "xhealth" => crate::xhealth::run_now;
    xjob_intervals_timeouts_and_recovery: "xjobs" => crate::xjobs::lifecycle;
    volumes_survive_restart: "volumes" => crate::apps::volumes;
    https_manual_certificates: "https-manual" => crate::https::certificates;
    https_acme_and_renewal: "https-acme" => crate::https::acme;
    insight_telemetry: "telemetry" => crate::telemetry::telemetry;
    proxy_access_records: "access" => crate::telemetry::access;
    #[ignore = "requires cluster"]
    node_failure_and_recovery: "recovery" => crate::nodes::recovery;
    node_membership_and_join_invitations: "membership" => crate::nodes::membership;
}
