//! `bee-dsl-sql` — Bee SQL DSL。
//!
//! 基于 Apache DataFusion 的 SQL parser / planner,扩展 `EMIT INTO` 与
//! `ASOF JOIN` 等流式语义,编译为 Bee `Dag`。
//!
//! S00 阶段仅占位;S13 起引入 DataFusion,S14 起实现 `EMIT INTO` 扩展,
//! S15 起实现 `ASOF JOIN` 与端到端 SQL → Dag 编译。

pub struct SqlParser;
