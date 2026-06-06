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
//!
//! S02 阶段:`echo` 需对接外部 echo server(本仓库尚未提供 `serve` 子命令,
//! 可用 `nc -l -k -p <port>` 或测试桩替代)。后续 story 逐步添加:
//! - S07: `run` 启动完整 Node
//! - S10: `run pipeline.sql` 跑硬编码 Pipeline
//! - S28: `jobs` / `inspect` / `diagnostics` / `cluster status` 等可观测子命令
//!
//! CLI 解析手动实现以遵守"零运行时外部依赖"约束
//! (仅 `tokio` + `bytes` + `bincode` 三件套)。

use std::env;
use std::process::ExitCode;

use bee_codec::{Frame, MessageType};
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
    println!("    echo <addr>      BRP echo client: send a Heartbeat Frame to <addr> and");
    println!("                    read the echo back (S02)");
}
