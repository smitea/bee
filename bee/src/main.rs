//! `bee` — Bee 节点主程序入口。
//!
//! 负责拉起一个 Node 进程:初始化控制面 / 数据面 / 运行时,加载 Plugin,
//! 启动 CLI 子命令入口。
//!
//! ## CLI 子命令
//! - `--version` / `-V` — 打印版本
//! - `--help` / `-h` — 打印帮助
//! - `echo <addr>` — BRP echo 客户端:连接 `<addr>`,发送 Heartbeat Frame,
//!   读回 echo,打印 `ok` 或失败原因
//! - `run <sql_file> [csv_file]` — 跑 SQL pipeline: 读 SQL,注册 CSV 作为
//!   源 `stream` 表,parse → analyze → physical plan → execute,打印结果。
//!   `csv_file` 缺省时按 `<sql_file_basename>.csv` 约定查找 (S15)。
//! - `diagnostics <task_id>` — 打印指定 Task 的 4 项 per-Phase 指标
//!   (S24)。MVP 占位:bee 独立二进制未持 worker,直接回退到说明信息。
//!   真实场景下 worker 在 Node 进程内,`diagnostics` 通过 admin RPC
//!   查询对应 Node (S28 wiring)。
//!
//! CLI 解析手动实现以遵守"零运行时外部依赖"约束
//! (仅 `tokio` + `bytes` + `bincode` 三件套)。

use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use bee_codec::{Frame, MessageType};
use bee_dsl_sql::run_pipeline;
use bee_transport::Connection;

const PKG_NAME: &str = env!("CARGO_PKG_NAME");
const PKG_VERSION: &str = env!("CARGO_PKG_VERSION");
const PKG_DESCRIPTION: &str = env!("CARGO_PKG_DESCRIPTION");

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version") | Some("-V") => {
            println!("{} {}", PKG_NAME, PKG_VERSION);
            ExitCode::SUCCESS
        }
        Some("--help") | Some("-h") | None => {
            print_help();
            ExitCode::SUCCESS
        }
        Some("echo") => match run_echo(args.get(1).map(String::as_str)).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{}: echo failed: {}", PKG_NAME, e);
                ExitCode::from(1)
            }
        },
        Some("run") => match run_pipeline_cli(
            args.get(1).map(String::as_str),
            args.get(2).map(String::as_str),
        )
        .await
        {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{}: run failed: {}", PKG_NAME, e);
                ExitCode::from(1)
            }
        },
        Some("diagnostics") => match run_diagnostics(args.get(1).map(String::as_str)) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{}: diagnostics failed: {}", PKG_NAME, e);
                ExitCode::from(1)
            }
        },
        Some(cmd) => {
            eprintln!("{}: unknown command `{}`", PKG_NAME, cmd);
            eprintln!("try `{} --help` for available commands", PKG_NAME);
            ExitCode::from(2)
        }
    }
}

async fn run_echo(addr: Option<&str>) -> Result<(), String> {
    let addr = addr.ok_or_else(|| "echo requires <addr>".to_string())?;
    let mut conn = Connection::connect(addr)
        .await
        .map_err(|e| format!("connect to {addr}: {e}"))?;
    let frame = Frame::new(MessageType::Heartbeat, 0, b"ping".to_vec());
    conn.send_frame(&frame)
        .await
        .map_err(|e| format!("send: {e}"))?;
    let echoed = conn
        .recv_frame()
        .await
        .map_err(|e| format!("recv: {e}"))?;
    if echoed == frame {
        println!("ok");
        Ok(())
    } else {
        Err(format!(
            "echoed frame does not match: got body={:?}",
            echoed.body
        ))
    }
}

async fn run_pipeline_cli(
    sql_path: Option<&str>,
    csv_path: Option<&str>,
) -> Result<(), String> {
    let sql_path = sql_path
        .ok_or_else(|| "run requires <sql_file>".to_string())?;
    let sql = std::fs::read_to_string(sql_path)
        .map_err(|e| format!("read {sql_path}: {e}"))?;
    let csv = match csv_path {
        Some(p) => PathBuf::from(p),
        None => derive_csv_path(Path::new(sql_path))
            .ok_or_else(|| "could not derive CSV path; pass [csv_file] explicitly".to_string())?,
    };
    let output = run_pipeline(&sql, &csv)
        .await
        .map_err(|e| format!("pipeline: {e}"))?;
    print!("{output}");
    Ok(())
}

/// Derive the default CSV path from the SQL path by replacing the
/// extension with `.csv` (e.g. `tests/data/simple_select.sql` →
/// `tests/data/simple_select.csv`).
fn derive_csv_path(sql_path: &Path) -> Option<PathBuf> {
    let stem = sql_path.file_stem()?.to_str()?;
    let parent = sql_path.parent().unwrap_or_else(|| Path::new("."));
    Some(parent.join(format!("{stem}.csv")))
}

/// S24 `bee diagnostics <task_id>` — MVP placeholder.
///
/// The `bee` binary doesn't run a worker itself; the worker lives
/// inside a Node process. Per the S24 acceptance criterion, the
/// real wiring is via an admin RPC that the Node exposes to query
/// its deployed Tasks' metrics (S28 wires this through the BRP
/// data channel). For the MVP, the CLI prints a clear message and
/// the `MetricsSnapshot` Display format the user can plug into a
/// future Node-bound binary.
fn run_diagnostics(task_id: Option<&str>) -> Result<(), String> {
    let id: u32 = task_id
        .ok_or_else(|| "diagnostics requires <task_id>".to_string())?
        .parse()
        .map_err(|e| format!("invalid task_id: {e}"))?;
    println!("task {id}:");
    println!("(diagnostics is available from the Node admin RPC; the");
    println!(" `bee` CLI does not host a worker. The library API at");
    println!(" bee_runtime::metrics::PhaseMetrics::snapshot prints the");
    println!(" four required fields:");
    println!(" - events_processed_total");
    println!(" - processing_latency_p50/p99 (histogram with");
    println!("   buckets 1ms / 10ms / 100ms / 1s / 10s)");
    println!(" - cpu_seconds_total");
    println!(" - backpressure_wait_seconds_total");
    println!();
    println!("Example Display output:");
    let example = bee_runtime::metrics::MetricsSnapshot {
        events_processed_total: 0,
        latency_count: 0,
        latency_avg: None,
        latency_p50: None,
        latency_p99: None,
        cpu_seconds_total: std::time::Duration::ZERO,
        backpressure_wait_seconds_total: std::time::Duration::ZERO,
    };
    print!("{example}");
    Ok(())
}

fn print_help() {
    println!("{} {} — {}", PKG_NAME, PKG_VERSION, PKG_DESCRIPTION);
    println!();
    println!("USAGE:");
    println!("    {} <COMMAND>", PKG_NAME);
    println!();
    println!("OPTIONS:");
    println!("    -V, --version    Print version and exit");
    println!("    -h, --help       Print this help and exit");
    println!();
    println!("COMMANDS:");
    println!("    echo <addr>                 BRP echo client: send a Heartbeat Frame to <addr>");
    println!("                               and read the echo back (S02)");
    println!("    run <sql_file> [csv_file]   Run a SQL pipeline: read the SQL, register the CSV");
    println!("                               as the `stream` source, parse → analyze → execute,");
    println!("                               and print the result table. csv_file defaults to");
    println!("                               <sql_file_basename>.csv (S15)");
    println!("    diagnostics <task_id>      Print the four per-Phase metrics for a Task (S24).");
    println!("                               MVP placeholder — the bee CLI does not host a");
    println!("                               worker; in production the Node admin RPC exposes");
    println!("                               this view (S28 wiring).");
}
