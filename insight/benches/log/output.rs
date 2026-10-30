#![expect(clippy::print_stdout)]

use std::path::Path;

use insight::error::{ErrorKind, Result};

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

pub(crate) fn step(number: usize, query: bool, label: &str) {
    println!("[{number}/{}] {label}", if query { 4 } else { 3 });
}

#[expect(clippy::cast_precision_loss)]
pub(crate) fn report(
    options: &super::Options,
    corpus: &super::data::Corpus,
    report: &super::Report,
) -> Result<()> {
    println!("\nResults");
    println!(
        "  Config          row group {} | flush {} ms | maintenance {} ms | seed {}",
        data_size(options.row_group_bytes as u64),
        super::WRITER_FLUSH_INTERVAL.as_millis(),
        insight::config::LOCAL_MAINTENANCE_DELAY.as_millis(),
        options.seed
    );
    println!(
        "  Corpus          {} | {} logs | {} bytes/log",
        data_size(corpus.raw_bytes),
        count(corpus.rows),
        corpus.raw_bytes / corpus.rows
    );
    println!(
        "  Input           {} indexes | {} logs",
        options.index_count,
        count(report.rows)
    );
    println!(
        "  Storage         {} raw | {} Parquet",
        data_size(report.bytes),
        data_size(report.parquet_bytes)
    );
    println!(
        "  Ingest          {:.3} s | {:.3} GiB/s",
        report.ingest.as_secs_f64(),
        report.bytes as f64 / GIB / report.ingest.as_secs_f64()
    );
    println!(
        "  Drain           {:.3} s (included in Ingest)",
        report.drain.as_secs_f64()
    );
    println!(
        "  Merge wait      {:.3} s (no eligible merges remain)",
        report.merge_wait.as_secs_f64()
    );
    println!(
        "  Ingest + merge  {:.3} s",
        report
            .ingest
            .saturating_add(report.merge_wait)
            .as_secs_f64()
    );
    if let Some(elapsed) = report.query {
        let count = options.index_count;
        println!(
            "  Query (AND)     {:.3} GiB/s raw | {count}/{count} verified",
            report.bytes as f64 / GIB / elapsed.as_secs_f64()
        );
    }
    if let Some(path) = &report.files {
        artifact(path);
    }
    if let Some(bytes) = report.server_peak_rss_bytes {
        println!(
            "  Server VmHWM    {} | PID {} (before process exit)",
            data_size(bytes),
            report.server_pid
        );
    }
    let nanos = |duration: std::time::Duration| {
        u64::try_from(duration.as_nanos()).map_err(|_| {
            ErrorKind::Invariant("benchmark duration overflowed u64 nanoseconds".to_owned())
        })
    };
    let result = serde_json::json!({
        "raw_bytes": report.bytes,
        "parquet_bytes": report.parquet_bytes,
        "rows": report.rows,
        "index_count": options.index_count,
        "seed": options.seed,
        "ingest_ns": nanos(report.ingest)?,
        "ingest_seconds": report.ingest.as_secs_f64(),
        "drain_ns": nanos(report.drain)?,
        "merge_ns": nanos(report.merge_wait)?,
        "query_ns": report.query.map(nanos).transpose()?,
        "query_verified": report.query.is_some(),
        "server_pid": report.server_pid,
        "server_peak_rss_bytes": report.server_peak_rss_bytes,
        "server_peak_rss_source": "proc_self_status_VmHWM_before_exit",
    });
    println!("BENCHMARK_RESULT {result}");
    Ok(())
}

pub(crate) fn artifact(path: &Path) {
    println!("  Files: {}", path.display());
}

#[expect(clippy::print_stderr)]
pub(crate) fn failure(seed: u64) {
    eprintln!("Benchmark failed (seed {seed})");
}

#[expect(clippy::cast_precision_loss)]
fn data_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.3} GiB", bytes as f64 / GIB)
    } else {
        format!("{:.3} MiB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn count(value: u64) -> String {
    let digits = value.to_string();
    let mut output = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(digit);
    }
    output
}
