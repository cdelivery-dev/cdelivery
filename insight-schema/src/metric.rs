use std::sync::{Arc, LazyLock};

use crate::sample::{ContainerSample, FilesystemSample, HostSample};
use arrow::array::{
    BooleanBuilder, StringDictionaryBuilder, TimestampNanosecondBuilder, UInt16Builder,
    UInt32Builder, UInt64Builder,
};
use arrow::datatypes::{DataType, Field, Schema, UInt32Type};
use arrow::record_batch::RecordBatch;

use crate::error::Result;

pub const BOOT_ID: &str = "boot_id";
pub const NODE_UPTIME_MS: &str = "node_uptime_ms";
pub const CPU_CORES: &str = "cpu_cores";
pub const CPU_USER_TICKS: &str = "cpu_user_ticks";
pub const CPU_SYSTEM_TICKS: &str = "cpu_system_ticks";
pub const CPU_IDLE_TICKS: &str = "cpu_idle_ticks";
pub const CPU_IOWAIT_TICKS: &str = "cpu_iowait_ticks";
pub const CPU_STEAL_TICKS: &str = "cpu_steal_ticks";
pub const MEMORY_TOTAL_BYTES: &str = "memory_total_bytes";
pub const MEMORY_AVAILABLE_BYTES: &str = "memory_available_bytes";
pub const MEMORY_FREE_BYTES: &str = "memory_free_bytes";
pub const SWAP_TOTAL_BYTES: &str = "swap_total_bytes";
pub const SWAP_FREE_BYTES: &str = "swap_free_bytes";
pub const PSI_CPU_SOME_USEC_TOTAL: &str = "psi_cpu_some_usec_total";
pub const PSI_MEMORY_SOME_USEC_TOTAL: &str = "psi_memory_some_usec_total";
pub const PSI_MEMORY_FULL_USEC_TOTAL: &str = "psi_memory_full_usec_total";
pub const PSI_IO_SOME_USEC_TOTAL: &str = "psi_io_some_usec_total";
pub const PSI_IO_FULL_USEC_TOTAL: &str = "psi_io_full_usec_total";
pub const NETWORK_INTERFACE_SET: &str = "network_interface_set";
pub const NETWORK_RECEIVE_BYTES_TOTAL: &str = "network_receive_bytes_total";
pub const NETWORK_TRANSMIT_BYTES_TOTAL: &str = "network_transmit_bytes_total";
pub const MOUNT_POINT: &str = "mount_point";
pub const TOTAL_BYTES: &str = "total_bytes";
pub const FREE_BYTES: &str = "free_bytes";
pub const AVAILABLE_BYTES: &str = "available_bytes";
pub const INODES_TOTAL: &str = "inodes_total";
pub const INODES_FREE: &str = "inodes_free";
pub const CONTAINER_STARTED_AT: &str = "container_started_at";
pub const CPU_QUOTA_USEC: &str = "cpu_quota_usec";
pub const CPU_PERIOD_USEC: &str = "cpu_period_usec";
pub const CPU_USAGE_USEC_TOTAL: &str = "cpu_usage_usec_total";
pub const CPU_THROTTLED_USEC_TOTAL: &str = "cpu_throttled_usec_total";
pub const MEMORY_LIMIT_BYTES: &str = "memory_limit_bytes";
pub const MEMORY_CURRENT_BYTES: &str = "memory_current_bytes";
pub const MEMORY_INACTIVE_FILE_BYTES: &str = "memory_inactive_file_bytes";
pub const MEMORY_ANON_BYTES: &str = "memory_anon_bytes";
pub const MEMORY_SWAP_CURRENT_BYTES: &str = "memory_swap_current_bytes";
pub const MEMORY_OOM_KILLS_TOTAL: &str = "memory_oom_kills_total";
pub const PIDS_CURRENT: &str = "pids_current";
pub const PIDS_MAX: &str = "pids_max";
pub const DISK_DEVICE_SET: &str = "disk_device_set";
pub const DISK_READ_BYTES_TOTAL: &str = "disk_read_bytes_total";
pub const DISK_WRITE_BYTES_TOTAL: &str = "disk_write_bytes_total";
pub const DISK_READ_OPS_TOTAL: &str = "disk_read_ops_total";
pub const DISK_WRITE_OPS_TOTAL: &str = "disk_write_ops_total";
pub const TCP_RETRANS_SEGS_TOTAL: &str = "tcp_retrans_segs_total";
pub const SERVICE_ID: &str = "service_id";
pub const SERVICE_NAME: &str = "service_name";
pub const MANAGED: &str = "managed";
pub const ELAPSED_MS: &str = "elapsed_ms";
pub const CPU_USAGE_USEC_DELTA: &str = "cpu_usage_usec_delta";
pub const CPU_THROTTLED_USEC_DELTA: &str = "cpu_throttled_usec_delta";
pub const MEMORY_OOM_KILLS_DELTA: &str = "memory_oom_kills_delta";
pub const PSI_CPU_SOME_USEC_DELTA: &str = "psi_cpu_some_usec_delta";
pub const PSI_MEMORY_SOME_USEC_DELTA: &str = "psi_memory_some_usec_delta";
pub const PSI_MEMORY_FULL_USEC_DELTA: &str = "psi_memory_full_usec_delta";
pub const PSI_IO_SOME_USEC_DELTA: &str = "psi_io_some_usec_delta";
pub const PSI_IO_FULL_USEC_DELTA: &str = "psi_io_full_usec_delta";
pub const DISK_READ_BYTES_DELTA: &str = "disk_read_bytes_delta";
pub const DISK_WRITE_BYTES_DELTA: &str = "disk_write_bytes_delta";
pub const DISK_READ_OPS_DELTA: &str = "disk_read_ops_delta";
pub const DISK_WRITE_OPS_DELTA: &str = "disk_write_ops_delta";
pub const NETWORK_RECEIVE_BYTES_DELTA: &str = "network_receive_bytes_delta";
pub const NETWORK_TRANSMIT_BYTES_DELTA: &str = "network_transmit_bytes_delta";
pub const TCP_RETRANS_SEGS_DELTA: &str = "tcp_retrans_segs_delta";

macro_rules! sample_batch {
    (
        $schema:ident, $builder:ident, $batch:ident, $sample:ty, |$s:ident| {
            $( $name:ident: $bty:ty = $field:expr => $method:ident($value:expr) ),+ $(,)?
        }
    ) => {
        pub static $schema: LazyLock<Arc<Schema>> =
            LazyLock::new(|| Arc::new(Schema::new(vec![$($field),+])));

        struct $builder {
            $( $name: $bty, )+
        }

        impl $builder {
            fn append(&mut self, $s: &$sample) {
                $( self.$name.$method($value); )+
            }
        }

        pub fn $batch(samples: &[$sample]) -> Result<RecordBatch> {
            let mut builder = $builder {
                $( $name: <$bty>::new(), )+
            };
            for sample in samples {
                builder.append(sample);
            }
            Ok(RecordBatch::try_new(
                $schema.clone(),
                vec![$( Arc::new(builder.$name.finish()) ),+],
            )?)
        }
    };
}

sample_batch! {
    HOST_METRIC_SCHEMA, HostBatchBuilder, host_batch, HostSample, |sample| {
        timestamp: TimestampNanosecondBuilder = super::nanos_field(super::TIMESTAMP)
            => append_value(crate::timestamp_nanos(sample.timestamp)),
        node_id: StringDictionaryBuilder<UInt32Type> = super::dict32(super::NODE_ID, false)
            => append_value(&sample.node_id),
        boot_id: StringDictionaryBuilder<UInt32Type> = super::dict32(BOOT_ID, false)
            => append_value(&sample.boot_id),
        node_uptime_ms: UInt64Builder = Field::new(NODE_UPTIME_MS, DataType::UInt64, false)
            => append_value(sample.node_uptime_ms),
        cpu_cores: UInt16Builder = Field::new(CPU_CORES, DataType::UInt16, false)
            => append_value(sample.cpu_cores),
        cpu_user_ticks: UInt64Builder = Field::new(CPU_USER_TICKS, DataType::UInt64, false)
            => append_value(sample.cpu_user_ticks),
        cpu_system_ticks: UInt64Builder = Field::new(CPU_SYSTEM_TICKS, DataType::UInt64, false)
            => append_value(sample.cpu_system_ticks),
        cpu_idle_ticks: UInt64Builder = Field::new(CPU_IDLE_TICKS, DataType::UInt64, false)
            => append_value(sample.cpu_idle_ticks),
        cpu_iowait_ticks: UInt64Builder = Field::new(CPU_IOWAIT_TICKS, DataType::UInt64, false)
            => append_value(sample.cpu_iowait_ticks),
        cpu_steal_ticks: UInt64Builder = Field::new(CPU_STEAL_TICKS, DataType::UInt64, false)
            => append_value(sample.cpu_steal_ticks),
        memory_total_bytes: UInt64Builder = Field::new(MEMORY_TOTAL_BYTES, DataType::UInt64, false)
            => append_value(sample.memory_total_bytes),
        memory_available_bytes: UInt64Builder = Field::new(MEMORY_AVAILABLE_BYTES, DataType::UInt64, true)
            => append_option(sample.memory_available_bytes),
        memory_free_bytes: UInt64Builder = Field::new(MEMORY_FREE_BYTES, DataType::UInt64, true)
            => append_option(sample.memory_free_bytes),
        memory_inactive_file_bytes: UInt64Builder = Field::new(MEMORY_INACTIVE_FILE_BYTES, DataType::UInt64, true)
            => append_option(sample.memory_inactive_file_bytes),
        swap_total_bytes: UInt64Builder = Field::new(SWAP_TOTAL_BYTES, DataType::UInt64, true)
            => append_option(sample.swap_total_bytes),
        swap_free_bytes: UInt64Builder = Field::new(SWAP_FREE_BYTES, DataType::UInt64, true)
            => append_option(sample.swap_free_bytes),
        psi_cpu_some_usec_total: UInt64Builder = Field::new(PSI_CPU_SOME_USEC_TOTAL, DataType::UInt64, true)
            => append_option(sample.psi_cpu_some_usec_total),
        psi_memory_some_usec_total: UInt64Builder = Field::new(PSI_MEMORY_SOME_USEC_TOTAL, DataType::UInt64, true)
            => append_option(sample.psi_memory_some_usec_total),
        psi_memory_full_usec_total: UInt64Builder = Field::new(PSI_MEMORY_FULL_USEC_TOTAL, DataType::UInt64, true)
            => append_option(sample.psi_memory_full_usec_total),
        psi_io_some_usec_total: UInt64Builder = Field::new(PSI_IO_SOME_USEC_TOTAL, DataType::UInt64, true)
            => append_option(sample.psi_io_some_usec_total),
        psi_io_full_usec_total: UInt64Builder = Field::new(PSI_IO_FULL_USEC_TOTAL, DataType::UInt64, true)
            => append_option(sample.psi_io_full_usec_total),
        network_interface_set: StringDictionaryBuilder<UInt32Type> = super::dict32(NETWORK_INTERFACE_SET, true)
            => append_option(sample.network_interface_set.as_deref()),
        network_receive_bytes_total: UInt64Builder = Field::new(NETWORK_RECEIVE_BYTES_TOTAL, DataType::UInt64, true)
            => append_option(sample.network_receive_bytes_total),
        network_transmit_bytes_total: UInt64Builder = Field::new(NETWORK_TRANSMIT_BYTES_TOTAL, DataType::UInt64, true)
            => append_option(sample.network_transmit_bytes_total),
    }
}

sample_batch! {
    FILESYSTEM_METRIC_SCHEMA, FilesystemBatchBuilder, filesystem_batch, FilesystemSample, |sample| {
        timestamp: TimestampNanosecondBuilder = super::nanos_field(super::TIMESTAMP)
            => append_value(crate::timestamp_nanos(sample.timestamp)),
        node_id: StringDictionaryBuilder<UInt32Type> = super::dict32(super::NODE_ID, false)
            => append_value(&sample.node_id),
        mount_point: StringDictionaryBuilder<UInt32Type> = super::dict32(MOUNT_POINT, false)
            => append_value(&sample.mount_point),
        total_bytes: UInt64Builder = Field::new(TOTAL_BYTES, DataType::UInt64, false)
            => append_value(sample.total_bytes),
        free_bytes: UInt64Builder = Field::new(FREE_BYTES, DataType::UInt64, false)
            => append_value(sample.free_bytes),
        available_bytes: UInt64Builder = Field::new(AVAILABLE_BYTES, DataType::UInt64, false)
            => append_value(sample.available_bytes),
        inodes_total: UInt64Builder = Field::new(INODES_TOTAL, DataType::UInt64, true)
            => append_option(sample.inodes_total),
        inodes_free: UInt64Builder = Field::new(INODES_FREE, DataType::UInt64, true)
            => append_option(sample.inodes_free),
    }
}

sample_batch! {
    CONTAINER_METRIC_SCHEMA, ContainerBatchBuilder, container_batch, ContainerSample, |sample| {
        timestamp: TimestampNanosecondBuilder = super::nanos_field(super::TIMESTAMP)
            => append_value(crate::timestamp_nanos(sample.timestamp)),
        node_id: StringDictionaryBuilder<UInt32Type> = super::dict32(super::NODE_ID, false)
            => append_value(&sample.node_id),
        boot_id: StringDictionaryBuilder<UInt32Type> = super::dict32(BOOT_ID, false)
            => append_value(&sample.boot_id),
        node_uptime_ms: UInt64Builder = Field::new(NODE_UPTIME_MS, DataType::UInt64, false)
            => append_value(sample.node_uptime_ms),
        elapsed_ms: UInt64Builder = Field::new(ELAPSED_MS, DataType::UInt64, true)
            => append_option(sample.elapsed_ms),
        service_id: StringDictionaryBuilder<UInt32Type> = super::dict32(SERVICE_ID, false)
            => append_value(&sample.service_id),
        service_name: StringDictionaryBuilder<UInt32Type> = super::dict32(SERVICE_NAME, false)
            => append_value(&sample.service_name),
        managed: BooleanBuilder = Field::new(MANAGED, DataType::Boolean, false)
            => append_value(sample.managed),
        container_id: StringDictionaryBuilder<UInt32Type> = super::dict32(super::CONTAINER_ID, false)
            => append_value(&sample.container_id),
        container_started_at: TimestampNanosecondBuilder = super::nanos_field(CONTAINER_STARTED_AT)
            => append_value(crate::timestamp_nanos(sample.container_started_at)),
        cpu_quota_usec: UInt64Builder = Field::new(CPU_QUOTA_USEC, DataType::UInt64, true)
            => append_option(sample.cpu_quota_usec),
        cpu_period_usec: UInt64Builder = Field::new(CPU_PERIOD_USEC, DataType::UInt64, true)
            => append_option(sample.cpu_period_usec),
        cpu_usage_usec_total: UInt64Builder = Field::new(CPU_USAGE_USEC_TOTAL, DataType::UInt64, true)
            => append_option(sample.cpu_usage_usec_total),
        cpu_throttled_usec_total: UInt64Builder = Field::new(CPU_THROTTLED_USEC_TOTAL, DataType::UInt64, true)
            => append_option(sample.cpu_throttled_usec_total),
        memory_limit_bytes: UInt64Builder = Field::new(MEMORY_LIMIT_BYTES, DataType::UInt64, true)
            => append_option(sample.memory_limit_bytes),
        memory_current_bytes: UInt64Builder = Field::new(MEMORY_CURRENT_BYTES, DataType::UInt64, true)
            => append_option(sample.memory_current_bytes),
        memory_inactive_file_bytes: UInt64Builder = Field::new(MEMORY_INACTIVE_FILE_BYTES, DataType::UInt64, true)
            => append_option(sample.memory_inactive_file_bytes),
        memory_anon_bytes: UInt64Builder = Field::new(MEMORY_ANON_BYTES, DataType::UInt64, true)
            => append_option(sample.memory_anon_bytes),
        memory_swap_current_bytes: UInt64Builder = Field::new(MEMORY_SWAP_CURRENT_BYTES, DataType::UInt64, true)
            => append_option(sample.memory_swap_current_bytes),
        memory_oom_kills_total: UInt64Builder = Field::new(MEMORY_OOM_KILLS_TOTAL, DataType::UInt64, true)
            => append_option(sample.memory_oom_kills_total),
        pids_current: UInt32Builder = Field::new(PIDS_CURRENT, DataType::UInt32, true)
            => append_option(sample.pids_current),
        pids_max: UInt32Builder = Field::new(PIDS_MAX, DataType::UInt32, true)
            => append_option(sample.pids_max),
        psi_cpu_some_usec_total: UInt64Builder = Field::new(PSI_CPU_SOME_USEC_TOTAL, DataType::UInt64, true)
            => append_option(sample.psi_cpu_some_usec_total),
        psi_memory_some_usec_total: UInt64Builder = Field::new(PSI_MEMORY_SOME_USEC_TOTAL, DataType::UInt64, true)
            => append_option(sample.psi_memory_some_usec_total),
        psi_memory_full_usec_total: UInt64Builder = Field::new(PSI_MEMORY_FULL_USEC_TOTAL, DataType::UInt64, true)
            => append_option(sample.psi_memory_full_usec_total),
        psi_io_some_usec_total: UInt64Builder = Field::new(PSI_IO_SOME_USEC_TOTAL, DataType::UInt64, true)
            => append_option(sample.psi_io_some_usec_total),
        psi_io_full_usec_total: UInt64Builder = Field::new(PSI_IO_FULL_USEC_TOTAL, DataType::UInt64, true)
            => append_option(sample.psi_io_full_usec_total),
        disk_device_set: StringDictionaryBuilder<UInt32Type> = super::dict32(DISK_DEVICE_SET, true)
            => append_option(sample.disk_device_set.as_deref()),
        disk_read_bytes_total: UInt64Builder = Field::new(DISK_READ_BYTES_TOTAL, DataType::UInt64, true)
            => append_option(sample.disk_read_bytes_total),
        disk_write_bytes_total: UInt64Builder = Field::new(DISK_WRITE_BYTES_TOTAL, DataType::UInt64, true)
            => append_option(sample.disk_write_bytes_total),
        disk_read_ops_total: UInt64Builder = Field::new(DISK_READ_OPS_TOTAL, DataType::UInt64, true)
            => append_option(sample.disk_read_ops_total),
        disk_write_ops_total: UInt64Builder = Field::new(DISK_WRITE_OPS_TOTAL, DataType::UInt64, true)
            => append_option(sample.disk_write_ops_total),
        network_interface_set: StringDictionaryBuilder<UInt32Type> = super::dict32(NETWORK_INTERFACE_SET, true)
            => append_option(sample.network_interface_set.as_deref()),
        network_receive_bytes_total: UInt64Builder = Field::new(NETWORK_RECEIVE_BYTES_TOTAL, DataType::UInt64, true)
            => append_option(sample.network_receive_bytes_total),
        network_transmit_bytes_total: UInt64Builder = Field::new(NETWORK_TRANSMIT_BYTES_TOTAL, DataType::UInt64, true)
            => append_option(sample.network_transmit_bytes_total),
        tcp_retrans_segs_total: UInt64Builder = Field::new(TCP_RETRANS_SEGS_TOTAL, DataType::UInt64, true)
            => append_option(sample.tcp_retrans_segs_total),
        cpu_usage_usec_delta: UInt64Builder = Field::new(CPU_USAGE_USEC_DELTA, DataType::UInt64, true)
            => append_option(sample.cpu_usage_usec_delta),
        cpu_throttled_usec_delta: UInt64Builder = Field::new(CPU_THROTTLED_USEC_DELTA, DataType::UInt64, true)
            => append_option(sample.cpu_throttled_usec_delta),
        memory_oom_kills_delta: UInt64Builder = Field::new(MEMORY_OOM_KILLS_DELTA, DataType::UInt64, true)
            => append_option(sample.memory_oom_kills_delta),
        psi_cpu_some_usec_delta: UInt64Builder = Field::new(PSI_CPU_SOME_USEC_DELTA, DataType::UInt64, true)
            => append_option(sample.psi_cpu_some_usec_delta),
        psi_memory_some_usec_delta: UInt64Builder = Field::new(PSI_MEMORY_SOME_USEC_DELTA, DataType::UInt64, true)
            => append_option(sample.psi_memory_some_usec_delta),
        psi_memory_full_usec_delta: UInt64Builder = Field::new(PSI_MEMORY_FULL_USEC_DELTA, DataType::UInt64, true)
            => append_option(sample.psi_memory_full_usec_delta),
        psi_io_some_usec_delta: UInt64Builder = Field::new(PSI_IO_SOME_USEC_DELTA, DataType::UInt64, true)
            => append_option(sample.psi_io_some_usec_delta),
        psi_io_full_usec_delta: UInt64Builder = Field::new(PSI_IO_FULL_USEC_DELTA, DataType::UInt64, true)
            => append_option(sample.psi_io_full_usec_delta),
        disk_read_bytes_delta: UInt64Builder = Field::new(DISK_READ_BYTES_DELTA, DataType::UInt64, true)
            => append_option(sample.disk_read_bytes_delta),
        disk_write_bytes_delta: UInt64Builder = Field::new(DISK_WRITE_BYTES_DELTA, DataType::UInt64, true)
            => append_option(sample.disk_write_bytes_delta),
        disk_read_ops_delta: UInt64Builder = Field::new(DISK_READ_OPS_DELTA, DataType::UInt64, true)
            => append_option(sample.disk_read_ops_delta),
        disk_write_ops_delta: UInt64Builder = Field::new(DISK_WRITE_OPS_DELTA, DataType::UInt64, true)
            => append_option(sample.disk_write_ops_delta),
        network_receive_bytes_delta: UInt64Builder = Field::new(NETWORK_RECEIVE_BYTES_DELTA, DataType::UInt64, true)
            => append_option(sample.network_receive_bytes_delta),
        network_transmit_bytes_delta: UInt64Builder = Field::new(NETWORK_TRANSMIT_BYTES_DELTA, DataType::UInt64, true)
            => append_option(sample.network_transmit_bytes_delta),
        tcp_retrans_segs_delta: UInt64Builder = Field::new(TCP_RETRANS_SEGS_DELTA, DataType::UInt64, true)
            => append_option(sample.tcp_retrans_segs_delta),
    }
}
