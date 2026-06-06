//! `bee-runtime` — Bee 数据面执行单元。
//!
//! 定义 [`Phase`] / [`Handler`] trait / [`Dag`] / [`Compiler`],并驱动单个 Task
//! 的输入-处理-输出循环与生命周期 (含 Checkpoint / Migrating)。
//!
//! S00 阶段仅占位;S03 起实现 `Handler` trait + `Phase` + `Dag` 1-Phase 版本,
//! S05 起支持多 Phase DAG, S10 起接入端到端 Pipeline 执行。

pub struct Phase;
