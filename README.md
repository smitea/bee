<div align="center">

# 🐝 Bee

**一个面向低延迟流式管线的 Rust 分布式数据流计算服务**  
**A Rust-native distributed dataflow pipeline compute service for low-latency stream pipelines**

`tokio` · `bincode` · 自研 BRP 二进制协议 / Custom BRP binary protocol

</div>

---

## 项目状态 / Project Status

> [!WARNING]
> **早期设计阶段（pre-alpha）**。本仓库目前仅含设计文档，尚未发布任何 crate、源码或二进制产物。接口与 crate 边界随时可能调整。
>
> **Early design stage (pre-alpha).** This repo currently ships design documents only. No crates, source, or binaries are published yet. APIs and crate boundaries are subject to change.

## 📚 文档导航 / Documentation Map

| 文档 | 内容 | 读者 |
| --- | --- | --- |
| [README.md](README.md) | 本文件：项目定位 + 路线图 | 所有 |
| [CONTEXT.md](CONTEXT.md) | 领域术语表（数据面 / 控制面 / Pipeline / Job / Task / …） | 所有 |
| [docs/product-design.md](docs/product-design.md) | 产品愿景 / 用户 / 场景 / 商业模式 | 产品 / 早期用户 / 合作方 |
| [docs/architecture.md](docs/architecture.md) | 完整技术架构 + 流程 + 4 个 mermaid 时序图 | 实现者 |
| [docs/stories.md](docs/stories.md) | **29 个实现 story（vertical slice 拆分）** | 实现者 |
| [docs/adr/](docs/adr/) | 不可逆架构决策记录 (ADR) | 实现者 |

## ✨ 设计目标 / Design Goals

| 中文 | English |
| --- | --- |
| 零运行时外部依赖（仅 `tokio` + `bytes` + `bincode`） | Zero runtime deps beyond `tokio` + `bytes` + `bincode` |
| 自研 BRP 二进制协议：15B 固定 Header + bincode Body | Custom BRP binary protocol: 15-byte fixed header + bincode body |
| 混合架构：数据面 P2P，控制面 Raft | Hybrid: Data Plane P2P, Control Plane Raft |
| `RequestID` 多路复用 + 滑动窗口背压 | `RequestID` multiplexing & sliding-window backpressure |
| DAG 驱动的 SQL / Lua DSL | DAG-driven SQL / Lua DSL |
| 限流数据源自动共享（Producer Pipeline 模式） | Auto-sharing of rate-limited datasources (Producer Pipeline pattern) |
| 插件一等公民（Handler / Adapter 动态库） | First-class plugin system (Handler / Adapter dynamic libraries) |
| 节点失效自动 Work-Stealing + Migrating | Automatic Work-Stealing + Migrating on node failure |

## 🧱 一句话架构 / Architecture in One Sentence

Bee 把用户写的 SQL/Lua Pipeline 编译成 Phase DAG，控制面用 Raft 仲裁"哪个 Job / Task 在哪个 Node 上跑"，数据面用 BRP 在 Node 之间 P2P 传输 Phase-to-Phase 业务流。任一节点挂掉时，Task 在 3× 心跳后被其他节点 Work-Stealing 接管；多个 Pipeline 共享限流数据源时共用 1 个 Producer Pipeline 而不是 N 个连接。

> 完整设计请参阅 [docs/architecture.md](docs/architecture.md) 与 [docs/product-design.md](docs/product-design.md)。  
> For full design, see [docs/architecture.md](docs/architecture.md) and [docs/product-design.md](docs/product-design.md).

## 📂 仓库结构 / Repository Layout

```
.
├── README.md              # 本文件 / This file
├── CONTEXT.md             # 领域术语表 / Domain glossary
└── docs/
    ├── product-design.md  # 产品设计文档 / Product design doc
    ├── architecture.md    # 技术架构详细设计 / Technical architecture
    └── adr/               # 架构决策记录 / Architecture Decision Records
        ├── 0001-data-plane-p2p-control-plane-raft.md
        ├── 0002-datasource-is-a-phase.md
        └── 0003-producer-pipeline-pattern.md
```

> 源码目录（`Cargo.toml`、`crates/` 等）将在路线图 0.1 阶段建立。  
> Source directories (`Cargo.toml`, `crates/`, …) will be created in roadmap stage 0.1.

## 🛣 路线图 / Roadmap

> 用户可见里程碑；详细技术里程碑见 [docs/architecture.md §11](docs/architecture.md#11-路线图)。

- [ ] **0.1 – 0.2** 单机能跑：本地 `bee run pipeline.sql`，能看到流。Demo 给种子用户。
- [ ] **0.3 – 0.4** 小集群：3 节点 Failover 演示。**第一个外部付费用户**。
- [ ] **0.5** 限流共享 + 跨 Pipeline：场景 A（量化）可上线，**第一个量化策略在生产**。
- [ ] **0.6 – 0.7** 插件系统（Rust 插件） + 调度优化器；**有 3 个外部 Adapter 在社区**。
- [ ] **0.8** SQL 性能调优：毫秒级微批 / UDF 性能分析 / Hint 语法；**量化场景可调**。
- [ ] **0.9 – 1.0** 生产可用：观测面板 + Schema 演进；**公开发布 1.0 公告**。
- [ ] **1.x** Enterprise 特性 + Lua runtime + 文档站 + 培训。
- [ ] **2.x** Managed Cloud 试点 + 插件市场上线（开放 C ABI + 多语言插件）。

## 🤝 贡献 / Contributing

项目尚处早期，欢迎通过以下方式参与：

- **设计讨论**：在 Issue 区提出协议 / 接口的质疑与替代方案。
- **实现贡献**：路线图 0.1 之后会开放首批 PR 通道。
- **文档改进**：直接提交 PR 修正本文档或 `docs/` 下的任一文件。

在公开贡献指南发布前，请遵循"先开 Issue 讨论，再提 PR"的原则。

## 📄 许可证 / License

待定 / TBD（将随首个 crate 发布时一并明确；倾向 Apache 2.0）。
