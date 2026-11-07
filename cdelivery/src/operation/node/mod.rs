pub(crate) mod typ;

use crate::error::Result;
use crate::{concept, usecase};

super::input::composite_input!(RelabelInput {
    #[garde(custom(super::validate::valid_node_id))]
    id: String,
    #[garde(dive)]
    changes: typ::UpdateNodeRequest,
});

super::input::composite_input!(MetricsInput {
    #[garde(custom(super::validate::valid_node_id))]
    id: String,
    #[garde(dive)]
    query: super::typ::metric::RangeQuery,
});

super::define!(
    List, super::input::Empty => typ::Nodes,
    "node_list", "List nodes with role, status, and labels.", Shared,
    super::Effect::Read, Standard, super::Media::Json;
    |app, _access, _input| {
        Ok(typ::Nodes {
            nodes: usecase::node::list(&app.docker)
                .await?
                .into_iter()
                .map(Into::into)
                .collect(),
        })
    }
);

super::define!(
    SilentAgents, super::input::Empty => typ::SilentAgents,
    "node_silent_agents", "List nodes with silent Insight agents. Last report times cover the past 15 minutes; null means no report in that window. Excludes Drain nodes and nodes created within 15 minutes that have not reported. Empty when Insight Agent is not deployed.", Shared,
    super::Effect::Read, Extended, super::Media::Json;
    |app, _access, _input, now| {
        Ok(typ::SilentAgents {
            silent_agents: usecase::node::silent_agents(&app.docker, &app.insight, now)
                .await?
                .into_iter()
                .map(|(node_id, last_report_at)| typ::SilentAgent {
                    node_id,
                    last_report_at,
                })
                .collect(),
        })
    }
);

super::define!(
    Get, super::input::Id => typ::Node,
    "node_get", "Get a node by ID.", Shared, super::Effect::Read,
    Standard, super::Media::Json;
    |app, _access, input| {
        Ok(usecase::node::get(&app.docker, &input.id).await?.into())
    }
);

super::define!(
    CreateJoinInvitation, typ::CreateJoinInvitationRequest => typ::JoinInvitation,
    "node_create_join_invitation", "Create a reusable 30-minute invitation for a Worker or Manager, invalidated when CDelivery restarts or upgrades. On the new node, run curl -fsSL https://cdelivery.dev/install.sh | sh -s -- --join '<join_url>' --token '<invitation>' using the returned values. Redemption requires the issuer's current authorization.", Admin,
    super::Effect::Write { destructive: false, idempotent: false },
    Standard, super::Media::Json;
    |app, access, input, now| {
        let (invitation, expires_at, manager_addr) = usecase::node::create_join_invitation(
            &app.docker,
            access,
            input.role.into(),
            now,
        )
        .await?;
        let address = std::net::SocketAddr::new(manager_addr.ip(), crate::config::PUBLIC_PORT.get());
        Ok(typ::JoinInvitation {
            invitation,
            expires_at,
            join_url: format!("http://{address}"),
        })
    }
);

super::define!(
    Relabel, RelabelInput => typ::Node,
    "node_relabel", "Replace all scheduling labels on a node.", Admin,
    super::Effect::Write { destructive: true, idempotent: false },
    Standard, super::Media::Json;
    |app, _access, input| {
        let labels = input
            .changes
            .labels
            .iter()
            .map(|label| label.parse::<concept::name::Name>())
            .collect::<Result<Vec<_>>>()?;
        Ok(usecase::node::relabel(&app.docker, &input.id, &labels)
            .await?
            .into())
    }
);

super::define!(
    Delete, super::input::Id => super::Done,
    "node_delete", "Force-remove a node without draining checks.", Admin,
    super::Effect::Write { destructive: true, idempotent: true },
    Standard, super::Media::Json;
    |app, _access, input| {
        usecase::node::delete(&app.docker, &input.id).await?;
        Ok(super::Done::new())
    }
);

super::define!(
    Metrics, MetricsInput => typ::NodeSeries,
    "node_metrics", "Query bucketed host and filesystem metrics. CPU utilization, cpu_user/system/iowait/steal, memory utilization, and psi_* use ratios 0-1; cpu_used/total use cores, memory and swap sizes bytes, network rates bytes/s over physical NICs, node_uptime seconds. Filesystems are local writable block-device filesystems, sampled every 5 minutes. Each (timestamp,mount_point) row has {min,avg,max}: utilization and inodes_utilization are ratios; *_bytes are bytes. step_secs = ceil(range/max_buckets), clamped 1s-1 day and raised to the Insight sampling interval when available; filesystem buckets are at least 300s. max_buckets defaults to 20, accepts 1-720; range <=31 days. Values are derived per sample before bucketing, preserving spikes. Missing buckets or fields mean no valid data.", Shared,
    super::Effect::Read, Extended, super::Media::Json;
    validate |input, now, report| {
        super::validate::append(
            report,
            garde::Path::new("query"),
            super::validate::query_time_range(input.query.start, input.query.end, now),
        );
    };
    |app, _access, input, now| {
        let (range, step_secs) = super::insight::metric_window(
            app,
            input.query.start,
            input.query.end,
            input.query.max_buckets,
            now,
        )
        .await;
        let (host, filesystems) =
            usecase::node::metrics(&app.docker, &app.insight, &input.id, range, step_secs).await?;
        Ok(typ::NodeSeries {
            step_secs,
            host,
            filesystems,
        })
    }
);
