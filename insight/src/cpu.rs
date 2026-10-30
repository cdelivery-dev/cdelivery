use std::collections::HashMap;

use datafusion::arrow::array::{
    Array, BooleanArray, DictionaryArray, RecordBatch, StringArray, TimestampNanosecondArray,
    UInt16Array, UInt64Array,
};
use datafusion::arrow::datatypes::UInt32Type;
use insight_schema::{CONTAINER_ID, NODE_ID, TIMESTAMP, metric};
use insight_wire::cpu::{ContainerSample, NodeSample, Response};
use jiff::Timestamp;

use crate::config;
use crate::error::{ErrorKind, Result};

#[derive(Debug, Default)]
pub(crate) struct Cache {
    services: scc::HashMap<String, HashMap<String, ContainerSample>>,
    nodes: scc::HashMap<String, NodeSample>,
}

fn valid_container_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn recent(at: Timestamp, now: Timestamp) -> bool {
    now.duration_since(at).abs() <= config::CPU_TTL
}

fn column<'a, T: Array + 'static>(batch: &'a RecordBatch, name: &str) -> Result<&'a T> {
    batch
        .column_by_name(name)
        .and_then(|array| array.as_any().downcast_ref())
        .ok_or_else(|| {
            ErrorKind::Invariant(format!("CPU column {name} has unexpected type")).into()
        })
}

fn dictionary<'a>(batch: &'a RecordBatch, name: &str) -> Result<impl Fn(usize) -> Option<&'a str>> {
    let ids = column::<DictionaryArray<UInt32Type>>(batch, name)?;
    let values = ids
        .values()
        .as_any()
        .downcast_ref::<StringArray>()
        .ok_or_else(|| ErrorKind::Invariant(format!("CPU column {name} has unexpected values")))?;
    Ok(move |row| {
        ids.key(row)
            .filter(|index| !values.is_null(*index))
            .map(|index| values.value(index))
    })
}

impl Cache {
    pub(crate) fn sweep(&self, now: Timestamp) {
        self.services.retain_sync(|_, samples| {
            samples.retain(|_, sample| recent(sample.observed_at, now));
            !samples.is_empty()
        });
        self.nodes
            .retain_sync(|_, sample| recent(sample.observed_at, now));
    }

    pub(crate) fn record_containers(&self, batch: &RecordBatch) -> Result<()> {
        let ids = dictionary(batch, CONTAINER_ID)?;
        let services = dictionary(batch, metric::SERVICE_ID)?;
        let managed = column::<BooleanArray>(batch, metric::MANAGED)?;
        let observed = column::<TimestampNanosecondArray>(batch, TIMESTAMP)?;
        let started = column::<TimestampNanosecondArray>(batch, metric::CONTAINER_STARTED_AT)?;
        let uptime = column::<UInt64Array>(batch, metric::NODE_UPTIME_MS)?;
        let usage = column::<UInt64Array>(batch, metric::CPU_USAGE_USEC_TOTAL)?;
        let now = Timestamp::now();
        for row in 0..batch.num_rows() {
            if !managed.value(row) {
                continue;
            }
            let (Some(id), Some(service)) = (ids(row), services(row)) else {
                continue;
            };
            let observed_at = Timestamp::from_nanosecond(i128::from(observed.value(row)))?;
            if !valid_container_id(id)
                || !insight_schema::valid_service_id(service)
                || !recent(observed_at, now)
            {
                continue;
            }
            let id = id.to_owned();
            let sample = ContainerSample {
                container_started_at: Timestamp::from_nanosecond(i128::from(started.value(row)))?,
                observed_at,
                node_uptime_ms: uptime.value(row),
                cpu_usage_usec_total: (!usage.is_null(row)).then(|| usage.value(row)),
            };
            let mut entry = self.services.entry_sync(service.to_owned()).or_default();
            let samples = entry.get_mut();
            if samples
                .get(&id)
                .is_none_or(|current| current.observed_at < observed_at)
            {
                samples.insert(id, sample);
            }
        }
        Ok(())
    }

    pub(crate) fn record_nodes(&self, batch: &RecordBatch) -> Result<()> {
        let ids = dictionary(batch, NODE_ID)?;
        let boots = dictionary(batch, metric::BOOT_ID)?;
        let observed = column::<TimestampNanosecondArray>(batch, TIMESTAMP)?;
        let uptime = column::<UInt64Array>(batch, metric::NODE_UPTIME_MS)?;
        let cores = column::<UInt16Array>(batch, metric::CPU_CORES)?;
        let user = column::<UInt64Array>(batch, metric::CPU_USER_TICKS)?;
        let system = column::<UInt64Array>(batch, metric::CPU_SYSTEM_TICKS)?;
        let idle = column::<UInt64Array>(batch, metric::CPU_IDLE_TICKS)?;
        let iowait = column::<UInt64Array>(batch, metric::CPU_IOWAIT_TICKS)?;
        let steal = column::<UInt64Array>(batch, metric::CPU_STEAL_TICKS)?;
        let now = Timestamp::now();
        for row in 0..batch.num_rows() {
            let (Some(id), Some(boot_id)) = (ids(row), boots(row)) else {
                continue;
            };
            let observed_at = Timestamp::from_nanosecond(i128::from(observed.value(row)))?;
            if !insight_wire::valid_node_id(id)
                || boot_id.is_empty()
                || cores.value(row) == 0
                || !recent(observed_at, now)
            {
                continue;
            }
            let sample = NodeSample {
                observed_at,
                boot_id: boot_id.to_owned(),
                node_uptime_ms: uptime.value(row),
                cpu_cores: cores.value(row),
                cpu_user_ticks: user.value(row),
                cpu_system_ticks: system.value(row),
                cpu_idle_ticks: idle.value(row),
                cpu_iowait_ticks: iowait.value(row),
                cpu_steal_ticks: steal.value(row),
            };
            match self.nodes.entry_sync(id.to_owned()) {
                scc::hash_map::Entry::Occupied(current) if current.observed_at >= observed_at => {}
                entry => {
                    entry.insert_entry(sample);
                }
            }
        }
        Ok(())
    }

    pub(crate) fn select(&self, ids: &[String]) -> Response {
        let now = Timestamp::now();
        let services = ids
            .iter()
            .filter_map(|id| {
                let samples: HashMap<_, _> = self.services.read_sync(id, |_, samples| {
                    samples
                        .iter()
                        .filter(|(_, sample)| recent(sample.observed_at, now))
                        .map(|(id, sample)| (id.clone(), sample.clone()))
                        .collect()
                })?;
                (!samples.is_empty()).then(|| (id.clone(), samples))
            })
            .collect();
        let mut nodes = HashMap::new();
        self.nodes.iter_sync(|id, sample| {
            if recent(sample.observed_at, now) {
                nodes.insert(id.clone(), sample.clone());
            }
            true
        });
        Response { services, nodes }
    }
}
