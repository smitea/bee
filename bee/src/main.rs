//! `bee` — Bee 节点主程序入口。
//!
//! 负责拉起一个 Node 进程:初始化控制面 / 数据面 / 运行时,加载 Plugin,
//! 启动 CLI 子命令入口。
//!
//! S00 阶段仅支持 `--version` / `--help`;后续 story 逐步添加:
//! - S02: `echo <addr>` BRP 端到端回环
//! - S07: `run` 启动完整 Node
//! - S10: `run pipeline.sql` 跑硬编码 Pipeline
//! - S28: `jobs` / `inspect` / `diagnostics` / `cluster status` 等可观测子命令
//!
//! CLI 解析手动实现以遵守"零运行时外部依赖"约束
//! (仅 `tokio` + `bytes` + `bincode` 三件套,tokio 在 S02 引入)。

use std::env;
use std::process::ExitCode;

const PKG_NAME: &str = env!("CARGO_PKG_NAME");
const PKG_VERSION: &str = env!("CARGO_PKG_VERSION");
const PKG_DESCRIPTION: &str = env!("CARGO_PKG_DESCRIPTION");

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version") | Some("-V") => {
            println!("{} {}", PKG_NAME, PKG_VERSION);
            ExitCode::SUCCESS
        }
        Some("--help") | Some("-h") => {
            print_help();
            ExitCode::SUCCESS
        }
        None => {
            print_help();
            ExitCode::SUCCESS
        }
        Some(cmd) => {
            eprintln!("{}: unknown command `{}`", PKG_NAME, cmd);
            eprintln!("try `{} --help` for available commands", PKG_NAME);
            ExitCode::from(2)
        }
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
    println!("    (none yet — S00 bootstrap)");
}
