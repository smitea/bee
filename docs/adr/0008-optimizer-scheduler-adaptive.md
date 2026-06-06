# 0008: Optimizer and Scheduler responsibilities; runtime adaptive optimization with MLFQ default

The user's original design mentioned "调度策略" and "优化器" in the same breath. We separate them into two layers and add a third:

- **Optimizer** (`bee-runtime` Compiler module): Compile-time DAG transformation. MVP: basic DAG optimization (Filter+Project fusion, cross-Node edge → same-Node edge folding, Producer/Subscriber affinity) + DataFusion's built-in SQL optimizer (ADR-0006). 1.x: cost-based optimization and statistics-driven reordering.

- **Scheduler** (`bee-control`): Cross-Node Task placement and rebalancing. MVP: simple bin-packing (resource-declaration vs Node capacity) + Work-Stealing. 0.7: feedback-driven rebalancing using per-Phase metrics (the user's "按 Phase 花费的时间" requirement).

- **Runtime Adaptive Optimization** (new layer, per user's Q8.2): A configurable **local scheduling policy** for intra-Node Task scheduling. Each Bee Node runs a "Runtime Scheduler" on top of tokio that decides which Task to poll next when multiple are ready. The default policy is **MLFQ (Multi-Level Feedback Queue)** with 3-4 priority queues and aging for starvation prevention. Alternative policies — **SJF**, **HRRN**, **SRTN** — are exposed via `bee.runtime.scheduler_policy` and selectable per deployment. The Runtime Scheduler observes per-Phase latency, throughput, and CPU usage; its policy uses these observations to bias which Task gets CPU share.

The three layers are clearly separated: Optimizer changes DAG structure at compile time, Scheduler places Tasks to Nodes at deploy/rebalance time, Runtime Adaptive Optimization decides CPU-share bias within a Node at runtime.

## Consequences

- The user's "按 Phase 花费的时间优化调度策略" requirement is satisfied by MLFQ default: short-Phases get higher priority, long-Phases get demoted, starving Phases get promoted. SJF/SRTN give strict shortest-first semantics; HRRN balances short jobs and long-waiting jobs.
- Adding a Runtime Scheduler layer on top of tokio adds complexity. We accept this for the control it gives over quant scenarios (per the user's Q6 performance emphasis).
- MLFQ in tokio is **cooperative**, not preemptive: it biases polling order rather than interrupting running tasks. Real preemption would need OS-level cgroup controls (1.x).
- The four policies (SJF / HRRN / MLFQ / SRTN) are exposed but not all are equally meaningful in cooperative scheduling. MLFQ is the natural default; SJF/SRTN work for predictable event arrivals (good for K-line ticks); HRRN is a fallback for mixed workloads.
- 0.7 closes the adaptive loop: cross-Node Scheduler consumes Runtime metrics and triggers rebalancing (move hot Phases to better Nodes).
