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
//! - `plugin list` — 列出 S33-deferred demo 的 5 个 mock plugin
//!   (binance / google_news / influxdb / mongodb / ta-lib),并报
//!   它们的 manifest(name / feature_version / abi_version /
//!   adapter & handler 数量)+ 对应 cdylib 构件是否已生成。MVP
//!   静态列出(PluginManager 尚未 wire 到 CLI);S34-S39 production
//!   plugin 上线后切到 libloading 实时清单。
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
            // S26: --measure + --replay. S29 wrap-up: --strict
            // (enables the use-directive preprocessor + strict
            // mode check).
            let mut positional: Vec<&str> = Vec::new();
            let mut measure = false;
            let mut replay: u32 = 1;
            let mut strict = false;
            for a in args.iter().skip(1).map(String::as_str) {
                match a {
                    "--measure" => measure = true,
                    "--replay" => replay = 2,
                    "--strict" => strict = true,
                    _ => positional.push(a),
                }
            }
            match run_pipeline_cli(
                positional.first().copied(),
                positional.get(1).copied(),
                measure,
                replay,
                strict,
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
        Some("plugin") => match run_plugin_cli(&args[1..]).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{}: plugin failed: {}", PKG_NAME, e);
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
    strict: bool,
) -> Result<(), String> {
    let sql_path = sql_path
        .ok_or_else(|| "run requires <sql_file>".to_string())?;
    let mut sql = std::fs::read_to_string(sql_path)
        .map_err(|e| format!("read {sql_path}: {e}"))?;
    // S29 wrap-up: when --strict is set, run the use-directive
    // preprocessor + strict-mode check. The preprocess library
    // surfaces a clear error if a `<adapter>.method(...)` call
    // isn't preceded by a matching `use <adapter>;` directive.
    if strict {
        let (directives, stripped) = bee_dsl_sql::preprocess(&sql)
            .map_err(|e| format!("strict-mode: {e}"))?;
        // S29 redo: also reject inline credentials in call args
        // and validate EMIT INTO targets. These run on the full
        // SQL (including the use directives) — the preprocessor
        // strips use lines internally.
        bee_dsl_sql::preprocess::check_inline_credentials(&sql)
            .map_err(|e| format!("strict-mode: {e}"))?;
        bee_dsl_sql::preprocess::check_emit_into(&sql, &directives)
            .map_err(|e| format!("strict-mode: {e}"))?;
        sql = stripped;
    }
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

/// S30 + S30 wrap-up: `bee secret put|get|list|delete
/// [--tenant <n>]`. MVP: in-process `InMemorySecretStore`.
/// `--tenant <n>` overrides the default tenant 0.
async fn run_secret_cli(args: &[String]) -> Result<(), String> {
    let store: Box<dyn SecretStore> = Box::new(InMemorySecretStore::new());
    let mut tenant: u16 = 0;
    // --tenant can appear anywhere; consume all occurrences.
    let mut filtered: Vec<String> = Vec::with_capacity(args.len());
    let mut iter = args.iter();
    while let Some(a) = iter.next() {
        if a == "--tenant" {
            tenant = iter
                .next()
                .ok_or_else(|| "--tenant requires an argument".to_string())?
                .parse()
                .map_err(|e: std::num::ParseIntError| format!("invalid tenant: {e}"))?;
        } else {
            filtered.push(a.clone());
        }
    }
    let sub = filtered.first().map(String::as_str);

    match sub {
        Some("put") => {
            let id = filtered
                .get(1)
                .ok_or_else(|| "secret put requires <id>".to_string())?;
            let mut value: Option<Vec<u8>> = None;
            let mut iter = filtered[2..].iter().map(String::as_str);
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
            let id = filtered
                .get(1)
                .ok_or_else(|| "secret get requires <id>".to_string())?;
            match store.get(tenant, id).map_err(|e| e.to_string())? {
                Some(v) => {
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
            let id = filtered
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

/// S31 + S29 wrap-up: `bee datasource list/inspect/create/
/// pause/resume/delete`. MVP: in-process DatasourceRegistry.
async fn run_datasource_cli(args: &[String]) -> Result<(), String> {
    let sub = args.first().map(String::as_str);
    let mut registry = DatasourceRegistry::new();
    seed_demo_registry(&mut registry);

    match sub {
        Some("list") => {
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
                        ds.name, ds.tenant, ds.adapter,
                        format!("{}", ds.status), ds.version_spec,
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
        Some("create") => {
            // S29 CLI follow-up: `bee datasource create <name>
            // --adapter <a> --plugin-version <v> --config <json>`.
            // MVP: plugin_id is derived from the adapter+version
            // string (deterministic hash for demo); production
            // resolves via PluginManager.
            let name = args
                .get(1)
                .ok_or_else(|| "datasource create requires <name>".to_string())?
                .to_string();
            let mut adapter: Option<String> = None;
            let mut version: Option<String> = None;
            let mut config: Option<String> = None;
            let mut tenant: u16 = 0;
            let mut iter = args[2..].iter().map(String::as_str);
            while let Some(a) = iter.next() {
                match a {
                    "--adapter" => {
                        adapter = Some(
                            iter.next()
                                .ok_or_else(|| "--adapter requires an argument".to_string())?
                                .to_string(),
                        );
                    }
                    "--plugin-version" => {
                        version = Some(
                            iter.next()
                                .ok_or_else(|| "--plugin-version requires an argument".to_string())?
                                .to_string(),
                        );
                    }
                    "--config" => {
                        config = Some(
                            iter.next()
                                .ok_or_else(|| "--config requires an argument".to_string())?
                                .to_string(),
                        );
                    }
                    "--tenant" => {
                        tenant = iter
                            .next()
                            .ok_or_else(|| "--tenant requires an argument".to_string())?
                            .parse()
                            .map_err(|e: std::num::ParseIntError| format!("invalid tenant: {e}"))?;
                    }
                    _ => return Err(format!("unknown flag `{a}`")),
                }
            }
            let adapter = adapter.ok_or_else(|| "datasource create requires --adapter".to_string())?;
            let version_str = version
                .ok_or_else(|| "datasource create requires --plugin-version".to_string())?;
            let config = config.unwrap_or_else(|| "{}".to_string());
            // S29 redo: validate the config against per-call-arg
            // rules before creating the Datasource.
            let cfg_json: serde_json::Value = serde_json::from_str(&config)
                .map_err(|e| format!("--config is not valid JSON: {e}"))?;
            bee_dsl_sql::preprocess::validate_datasource_config(&cfg_json)
                .map_err(|e| format!("datasource create: {e}"))?;
            let version_spec = bee_plugin_sdk::VersionSpec::parse(&version_str)
                .map_err(|e| format!("invalid plugin-version: {e}"))?;
            let plugin_id = compute_plugin_id(format!("{adapter}@{version_str}").as_bytes());
            let ds = Datasource::new(name.clone(), tenant, adapter, plugin_id, version_spec, config);
            registry
                .create(ds)
                .map_err(|e| format!("datasource create: {e}"))?;
            println!("datasource {name} created");
            Ok(())
        }
        Some("pause") => {
            let name = args
                .get(1)
                .ok_or_else(|| "datasource pause requires <name>".to_string())?;
            // S29 redo: pause also triggers Draining on referencing
            // Jobs. Print the Draining event so the operator can see
            // which Jobs will be migrated.
            let ev = registry
                .pause(0, name)
                .map_err(|e| format!("datasource pause: {e}"))?;
            println!("datasource {name} paused");
            if let Some(ev) = ev {
                println!(
                    "  Draining triggered: {} referencing job(s) [{}]",
                    ev.referencing_jobs.len(),
                    ev.referencing_jobs
                        .iter()
                        .map(|j| j.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            } else {
                println!("  (no referencing jobs — no Draining needed)");
            }
            Ok(())
        }
        Some("test") => {
            // S29 redo: `bee datasource test <name>`. MVP: run a
            // stub probe that validates the Datasource exists, the
            // config is valid JSON, and (if a Plugin manifest is
            // present) the Plugin's `test_connection` returns Ok.
            // Production wires this through libloading.
            let name = args
                .get(1)
                .ok_or_else(|| "datasource test requires <name>".to_string())?;
            let ds = registry
                .get(0, name)
                .ok_or_else(|| format!("datasource `{name}` not found in tenant 0"))?;
            println!("probing datasource `{name}`...");
            println!("  adapter:    {}", ds.adapter);
            println!("  plugin_id:  {}", ds.plugin_id);
            println!("  version:    {}", ds.version_spec);
            // Validate the stored config is still valid JSON + no
            // per-call args. This catches drift after edits.
            let cfg: serde_json::Value = serde_json::from_str(&ds.config)
                .map_err(|e| format!("stored config is invalid JSON: {e}"))?;
            bee_dsl_sql::preprocess::validate_datasource_config(&cfg)
                .map_err(|e| format!("stored config rejected: {e}"))?;
            // Record a successful probe (in production this is the
            // result of the Plugin's test_connection call).
            registry
                .record_probe_success(0, name)
                .map_err(|e| format!("probe: {e}"))?;
            println!("  result:     ok (stub probe — production wires Plugin::test_connection)");
            Ok(())
        }
        Some("resume") => {
            let name = args
                .get(1)
                .ok_or_else(|| "datasource resume requires <name>".to_string())?;
            registry
                .resume(0, name)
                .map_err(|e| format!("datasource resume: {e}"))?;
            println!("datasource {name} resumed");
            Ok(())
        }
        Some("delete") => {
            let name = args
                .get(1)
                .ok_or_else(|| "datasource delete requires <name>".to_string())?;
            registry
                .delete(0, name)
                .map_err(|e| format!("datasource delete: {e}"))?;
            println!("datasource {name} deleted");
            Ok(())
        }
        Some(other) => Err(format!("unknown datasource subcommand `{other}`")),
        None => Err(
            "datasource requires a subcommand: list|inspect|create|pause|resume|delete|test"
                .to_string(),
        ),
    }
}

async fn run_plugin_cli(args: &[String]) -> Result<(), String> {
    // S33-deferred `bee plugin list`. MVP: print the 5 mock plugins
    // that ship in `plugins/` (the PluginManager is unit-tested but
    // not yet wired to a CLI; S34-S39 production plugins will swap
    // the static list for a libloading-driven scan).
    match args.first().map(String::as_str) {
        Some("list") => {
            // Crate name -> (logical plugin name, kind).
            // `kind` is "input" / "output" / "handler" — the mock
            // plugin set covers all three Adapter kinds + a
            // Handler-only plugin (ta-lib).
            let entries: &[(&str, &str, &str)] = &[
                ("bee-plugin-binance", "binance", "input"),
                ("bee-plugin-google-news", "google_news", "input"),
                ("bee-plugin-influxdb", "influxdb", "output"),
                ("bee-plugin-mongodb", "mongodb", "output"),
                ("bee-plugin-ta-lib", "ta-lib", "handler"),
            ];
            // The mock plugins all declare feature_version=1.0.0 /
            // abi_version=v1. S33 deferred the per-plugin version
            // surface to S34; once production plugins land, this
            // resolves through the loaded manifest.
            println!(
                "{:<30} | {:<12} | {:<5} | {:<5} | {:<7} | {}",
                "crate", "name", "ver", "abi", "kind", "artifact"
            );
            println!("{}", "-".repeat(76));
            for (crate_name, logical_name, kind) in entries {
                let lib_stem = crate_name.replace('-', "_");
                let artifact = if cfg!(target_os = "macos") {
                    format!("target/debug/lib{lib_stem}.dylib")
                } else {
                    format!("target/debug/lib{lib_stem}.so")
                };
                let status = if std::path::Path::new(&artifact).exists() {
                    "built"
                } else {
                    "missing"
                };
                println!(
                    "{:<30} | {:<12} | {:<5} | {:<5} | {:<7} | {}",
                    crate_name, logical_name, "1.0.0", "v1", kind, status,
                );
            }
            Ok(())
        }
        Some(other) => Err(format!("unknown plugin subcommand `{other}`")),
        None => Err("plugin requires a subcommand: list".to_string()),
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
    println!("      [--strict]                and print the result table. csv_file defaults to");
    println!("                               <sql_file_basename>.csv. --measure prints");
    println!("                               per-iteration latency (S26); --replay N runs the");
    println!("                               plan N times to simulate a micro-batch loop.");
    println!("                               --strict enables the use-directive preprocessor");
    println!("                               + strict-mode check (S29); e.g. `use binance;`");
    println!("                               must precede `binance.subscribe(...)` or the");
    println!("                               pipeline fails to compile.");
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
    println!("    datasource create <name>   Register a Datasource (S29 acceptance). Required:");
    println!("        --adapter <a>         adapter name (e.g., 'binance').");
    println!("        --plugin-version <v>  SemVer range ('1.4.2', '^1.0', '~1.2', 'latest').");
    println!("        [--config <json>]      Adapter-specific config (default: empty JSON).");
    println!("        [--tenant <n>]         Tenant (default 0 = global).");
    println!("    datasource inspect <name> Show Datasource header + per-Datasource health");
    println!("                                  (S31 acceptance: Producer Node, plugin_id, version,");
    println!("                                  health metrics, referencing Job count).");
    println!("    datasource pause <name>   Pause the Datasource (S31: triggers Draining on");
    println!("                                  referencing Jobs in production; MVP: status flag).");
    println!("    datasource resume <name>  Resume a paused Datasource.");
    println!("    datasource delete <name>  Remove the Datasource entry.");
    println!("    plugin list                List the S33-deferred mock plugins (binance /");
    println!("                              google_news / influxdb / mongodb / ta-lib) and");
    println!("                              report whether each cdylib artifact is built");
    println!("                              (target/debug/lib<name>.dylib or .so). MVP static");
    println!("                              list; S34-S39 swaps in a libloading-driven scan.");
}
