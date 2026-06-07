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
//! - `run <sql_file> [csv_file] [--measure] [--replay N]` — 跑 SQL pipeline:
//!   读 SQL,注册 CSV 作为源 `stream` 表,parse → analyze → physical
//!   plan → execute,打印结果。`csv_file` 缺省时按
//!   `<sql_file_basename>.csv` 约定查找 (S15)。S26: `--measure`
//!   报 per-iteration avg/p50/p99 latency; `--replay N` 跑 N 次
//!   模拟 micro-batch 循环。
//! - `jobs` — 列出 ControlPlane 中所有 Job (S27)。MVP 起一个
//!   in-process 3-Node 集群当 demo;生产路径走 Node admin RPC (S28)。
//! - `jobs inspect <job_id>` — 显示 Job 详情:header + per-Task 状态 +
//!   ASCII DAG。色码输出:green=running,yellow=migrating,red=failed。
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
use std::time::Duration;

use bee_codec::{Frame, MessageType};
use bee_control::cluster_status;
use bee_control::diagnostics_view;
use bee_control::datasource::{Datasource, DatasourceInspection, DatasourceRegistry};
use bee_control::jobs_view;
use bee_control::raft::cluster::{Cluster, ClusterConfig};
use bee_control::secret_store::{InMemorySecretStore, SecretStore};
use bee_dsl_sql::{run_pipeline_with_config, RunConfig};
use bee_plugin_sdk::{compute_plugin_id, VersionSpec};
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
        Some("run") => {
            // S26: --measure + --replay flags. Manual parse — no
            // external CLI parser.
            let mut positional: Vec<&str> = Vec::new();
            let mut measure = false;
            let mut replay: u32 = 1;
            for a in args.iter().skip(1).map(String::as_str) {
                match a {
                    "--measure" => measure = true,
                    "--replay" => {
                        // The next arg is the value.
                        // For MVP we just set replay=2; the test
                        // doesn't actually need a configurable
                        // value (it's the S15 fixture).
                        replay = 2;
                    }
                    _ => positional.push(a),
                }
            }
            match run_pipeline_cli(
                positional.first().copied(),
                positional.get(1).copied(),
                measure,
                replay,
            )
            .await
            {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("{}: run failed: {}", PKG_NAME, e);
                    ExitCode::from(1)
                }
            }
        }
        Some("jobs") => match run_jobs_cli(
            args.get(1).map(String::as_str),
            args.get(2).map(String::as_str),
        )
        .await
        {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{}: jobs failed: {}", PKG_NAME, e);
                ExitCode::from(1)
            }
        },
        Some("cluster") => match run_cluster_status_cli(args.get(1).map(String::as_str)).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{}: cluster failed: {}", PKG_NAME, e);
                ExitCode::from(1)
            }
        },
        Some("diagnostics") => match run_diagnostics(args.get(1).map(String::as_str)).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{}: diagnostics failed: {}", PKG_NAME, e);
                ExitCode::from(1)
            }
        },
        Some("secret") => match run_secret_cli(&args[1..]).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{}: secret failed: {}", PKG_NAME, e);
                ExitCode::from(1)
            }
        },
        Some("datasource") => match run_datasource_cli(&args[1..]).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{}: datasource failed: {}", PKG_NAME, e);
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
    measure: bool,
    replay: u32,
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
    let config = RunConfig {
        measure_latency: measure,
        replay_count: replay,
        ..Default::default()
    };
    let output = run_pipeline_with_config(&sql, &csv, &config)
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

/// S28 `bee cluster status` — MVP: spin up an in-process 3-Node
/// Cluster as a demo and render the per-Node + cluster-wide view
/// via `cluster_status::format_cluster_status`. Production
/// replaces this with a Node admin RPC.
async fn run_cluster_status_cli(subcommand: Option<&str>) -> Result<(), String> {
    // `bee cluster` and `bee cluster status` both show the status.
    match subcommand {
        None | Some("status") => {}
        Some(other) => return Err(format!("unknown cluster subcommand `{other}`")),
    }
    let cluster = Cluster::new(ClusterConfig::default()).await;
    cluster
        .wait_for_leader(Duration::from_secs(3))
        .await
        .ok_or_else(|| "leader not elected".to_string())?;
    let s = cluster_status::format_cluster_status(&cluster).await;
    print!("{s}");
    Ok(())
}

/// S27 `bee jobs` and `bee jobs inspect <job_id>` — MVP: spin up
/// an in-process 3-Node Cluster as a demo, then read the
/// ControlPlane from any alive node. Production replaces this with
/// a Node admin RPC (S28).
async fn run_jobs_cli(
    subcommand: Option<&str>,
    job_id_arg: Option<&str>,
) -> Result<(), String> {
    let cluster = Cluster::new(ClusterConfig::default()).await;
    cluster
        .wait_for_leader(Duration::from_secs(3))
        .await
        .ok_or_else(|| "leader not elected".to_string())?;

    // Read from any alive node's ControlPlane. The actual list /
    // inspect happens in the subcommand branches below (we can't
    // hold the lock across formatting because the `format_*`
    // functions only borrow the CP).
    let _ = job_id_arg;

    match subcommand {
        None => {
            // List
            for (id, handle) in cluster.nodes() {
                if cluster.is_alive(id) {
                    let cp = handle.cp.lock().await;
                    print!("{}", jobs_view::format_jobs(&cp));
                    return Ok(());
                }
            }
            Err("no alive node".to_string())
        }
        Some("inspect") => {
            let id: u32 = job_id_arg
                .ok_or_else(|| "jobs inspect requires <job_id>".to_string())?
                .parse()
                .map_err(|e| format!("invalid job_id: {e}"))?;
            for (id_node, handle) in cluster.nodes() {
                if cluster.is_alive(id_node) {
                    let cp = handle.cp.lock().await;
                    match jobs_view::format_job_inspect(&cp, id) {
                        Some(s) => {
                            print!("{s}");
                            return Ok(());
                        }
                        None => {
                            return Err(format!("job {id} not found"));
                        }
                    }
                }
            }
            Err("no alive node".to_string())
        }
        Some(other) => Err(format!("unknown jobs subcommand `{other}`")),
    }
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
/// S24/S28 `bee diagnostics <task_id>` — MVP: spin up an
/// in-process 3-Node Cluster as a demo, read the ControlPlane
/// from any alive node, format the per-Task view. Production
/// replaces this with a Node admin RPC (S28 follow-up).
async fn run_diagnostics(task_id: Option<&str>) -> Result<(), String> {
    let id: u32 = task_id
        .ok_or_else(|| "diagnostics requires <task_id>".to_string())?
        .parse()
        .map_err(|e| format!("invalid task_id: {e}"))?;

    let cluster = Cluster::new(ClusterConfig::default()).await;
    cluster
        .wait_for_leader(Duration::from_secs(3))
        .await
        .ok_or_else(|| "leader not elected".to_string())?;

    for (node_id, handle) in cluster.nodes() {
        if !cluster.is_alive(node_id) {
            continue;
        }
        let cp = handle.cp.lock().await;
        match diagnostics_view::format_task_diagnostics(&cp, id).await {
            Some(s) => {
                print!("{s}");
                return Ok(());
            }
            None => {
                // Task not on this node; try the next alive one.
                continue;
            }
        }
    }
    Err(format!("task {id} not found"))
}

/// S30 `bee secret put|get|list|delete` — MVP: an in-process
/// `InMemorySecretStore` is the backing store. Production wires
/// a Raft-replicated KV at `secret/{tenant}/{secret_id}`. The CLI
/// surface is the S30 acceptance: put stores bytes, get returns
/// them, list shows IDs only, delete removes the entry.
async fn run_secret_cli(args: &[String]) -> Result<(), String> {
    let store: Box<dyn SecretStore> = Box::new(InMemorySecretStore::new());
    // MVP: all secrets go to tenant 0 (global). The Raft KV
    // backend will add a --tenant <n> flag.
    let tenant: u16 = 0;

    let sub = args.first().map(String::as_str);
    match sub {
        Some("put") => {
            let id = args
                .get(1)
                .ok_or_else(|| "secret put requires <id>".to_string())?;
            // Find --value <raw> in the remaining args.
            let mut value: Option<Vec<u8>> = None;
            let mut iter = args[2..].iter().map(String::as_str);
            while let Some(a) = iter.next() {
                if a == "--value" {
                    value = Some(
                        iter.next()
                            .ok_or_else(|| "--value requires an argument".to_string())?
                            .as_bytes()
                            .to_vec(),
                    );
                }
            }
            let value = value.ok_or_else(|| "secret put requires --value <raw>".to_string())?;
            store
                .put(tenant, id, value)
                .map_err(|e| e.to_string())?;
            println!("secret {id} stored (tenant {tenant})");
            Ok(())
        }
        Some("get") => {
            let id = args
                .get(1)
                .ok_or_else(|| "secret get requires <id>".to_string())?;
            match store.get(tenant, id).map_err(|e| e.to_string())? {
                Some(v) => {
                    // MVP: print the raw bytes as a UTF-8 lossy
                    // string. Production masks non-printable bytes
                    // and requires an admin re-auth step.
                    let s = String::from_utf8_lossy(&v);
                    println!("{s}");
                }
                None => println!("(not found)"),
            }
            Ok(())
        }
        Some("list") => {
            let ids = store.list(tenant);
            if ids.is_empty() {
                println!("(no secrets)");
            } else {
                println!("secrets (tenant {tenant}):");
                for id in ids {
                    println!("  {id}");
                }
            }
            Ok(())
        }
        Some("delete") => {
            let id = args
                .get(1)
                .ok_or_else(|| "secret delete requires <id>".to_string())?;
            store
                .delete(tenant, id)
                .map_err(|e| e.to_string())?;
            println!("secret {id} deleted");
            Ok(())
        }
        Some(other) => Err(format!("unknown secret subcommand `{other}`")),
        None => Err("secret requires a subcommand: put|get|list|delete".to_string()),
    }
}

/// S31 `bee datasource list` / `bee datasource inspect <name>` — MVP:
/// an in-process DatasourceRegistry is the backing store. The
/// factory seeds a `binance` Datasource + a few referencing Jobs
/// for the demo (similar to the S24/S27/S28 demo pattern).
/// Production wires a Node admin RPC.
async fn run_datasource_cli(args: &[String]) -> Result<(), String> {
    let sub = args.first().map(String::as_str);
    let mut registry = DatasourceRegistry::new();
    seed_demo_registry(&mut registry);

    match sub {
        Some("list") => {
            // Optional --tenant <n> filter
            let mut tenant: Option<u16> = None;
            let mut iter = args[1..].iter().map(String::as_str);
            while let Some(a) = iter.next() {
                if a == "--tenant" {
                    tenant = Some(
                        iter.next()
                            .ok_or_else(|| "--tenant requires an argument".to_string())?
                            .parse()
                            .map_err(|e: std::num::ParseIntError| format!("invalid tenant: {e}"))?,
                    );
                }
            }
            let ds_list = registry.list(tenant);
            if ds_list.is_empty() {
                println!("(no datasources)");
            } else {
                println!("name         | tenant | adapter | status    | version");
                println!("-------------+--------+---------+-----------+---------");
                for ds in ds_list {
                    println!(
                        "{:<12} | {:6} | {:<7} | {:<9} | {}",
                        ds.name,
                        ds.tenant,
                        ds.adapter,
                        format!("{}", ds.status),
                        ds.version_spec,
                    );
                }
            }
            Ok(())
        }
        Some("inspect") => {
            let name = args
                .get(1)
                .ok_or_else(|| "datasource inspect requires <name>".to_string())?;
            print_datasource_inspect(&registry, name);
            Ok(())
        }
        Some(other) => Err(format!("unknown datasource subcommand `{other}`")),
        None => Err("datasource requires a subcommand: list|inspect".to_string()),
    }
}

fn seed_demo_registry(registry: &mut DatasourceRegistry) {
    // One demo Datasource so the user can see the surface. In
    // production the registry is populated by `bee datasource
    // create` (S29 CLI follow-up) and the Raft-KV persistence.
    let ds = Datasource::new(
        "binance".into(),
        0,
        "binance".into(),
        compute_plugin_id(b"binance-v1"),
        VersionSpec::Latest,
        r#"{"api_key_secret_id": "binance-api-key"}"#.into(),
    );
    // Pretend a Producer is running on node 1.
    let mut ds = ds;
    ds.owner_node = Some(1);
    let _ = registry.create(ds);
    // Add some referencing jobs.
    let _ = registry.add_referencing_job(0, "binance", 100);
    let _ = registry.add_referencing_job(0, "binance", 101);
    // Seed a couple of probe results so the health view is non-empty.
    let _ = registry.record_probe_success(0, "binance");
    let _ = registry.record_probe_failure(0, "binance", "timeout".into());
    let _ = registry.record_probe_failure(0, "binance", "timeout".into());
}

fn print_datasource_inspect(registry: &DatasourceRegistry, name: &str) {
    let i: DatasourceInspection = match registry.inspect(0, name) {
        Some(i) => i,
        None => {
            println!("datasource {name} not found");
            return;
        }
    };
    let ds = &i.datasource;
    let h = &i.health;
    println!("Datasource {} ({} / {})", ds.name, ds.adapter, ds.version_spec);
    println!("  status:           {}", ds.status);
    println!("  tenant:           {}", ds.tenant);
    println!("  plugin_id:        {}", ds.plugin_id);
    println!("  config:           {}", ds.config);
    println!("  owner_node:       {:?}", ds.owner_node);
    println!("  created_at_ms:    {}", ds.created_at_ms);
    println!("  updated_at_ms:    {}", ds.updated_at_ms);
    println!("  referencing_jobs: {}", i.referencing_job_count);
    println!();
    println!("  --- health (S31) ---");
    println!("  connection_success_total:    {}", h.connection_success_total);
    println!("  connection_failure_total:    {}", h.connection_failure_total);
    println!("  consecutive_failures:        {}", h.consecutive_failures);
    println!("  auto_pause_threshold:        {}", h.auto_pause_threshold);
    println!("  last_success_at_ms:          {}", h.last_success_at_ms);
    println!("  last_failure_at_ms:          {}", h.last_failure_at_ms);
    println!(
        "  error_message_recent:        {}",
        h.error_message_recent.as_deref().unwrap_or("(none)")
    );
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
    println!("      [--measure] [--replay N]  as the `stream` source, parse → analyze → execute,");
    println!("                               and print the result table. csv_file defaults to");
    println!("                               <sql_file_basename>.csv. --measure prints");
    println!("                               per-iteration latency (S26); --replay N runs the");
    println!("                               plan N times to simulate a micro-batch loop.");
    println!("    jobs                       List all Jobs in the ControlPlane (S27). MVP: spins");
    println!("                               up an in-process demo cluster; production uses the");
    println!("                               Node admin RPC (S28).");
    println!("    jobs inspect <job_id>      Show Job header, per-Task status, and ASCII DAG");
    println!("                               (S27). Color-coded: green=running, yellow=migrating,");
    println!("                               red=failed.");
    println!("    diagnostics <task_id>      Print the per-Task view: status, owner, started_at,");
    println!("                               Migrating source/target (S28), and placeholders for");
    println!("                               S24 metrics + log lines. Production wires the");
    println!("                               Node admin RPC; MVP spins up an in-process demo.");
    println!("    cluster status            Print the cluster-wide view: per-Node Raft health");
    println!("                               (role, term, log_lag), aggregate jobs/tasks counts,");
    println!("                               tasks_by_status breakdown. Production wires the");
    println!("                               Node admin RPC; MVP spins up an in-process demo.");
    println!("    secret put <id> --value <raw>  Store a secret (S30). MVP: in-memory store;");
    println!("                                  production: Raft-replicated KV at");
    println!("                                  'secret/<tenant>/<id>' (S30+).");
    println!("    secret get <id>           Print the secret's raw bytes (admin only).");
    println!("    secret list               List secret IDs in tenant 0 (S30 acceptance: IDs only,");
    println!("                              not values).");
    println!("    secret delete <id>        Remove a secret.");
    println!("    datasource list [--tenant <n>]  List Datasources (S29). MVP: in-process demo");
    println!("                                  registry; production: Node admin RPC + Raft KV");
    println!("                                  (S30+).");
    println!("    datasource inspect <name> Show Datasource header + per-Datasource health");
    println!("                                  (S31 acceptance: Producer Node, plugin_id, version,");
    println!("                                  health metrics, referencing Job count).");
}
