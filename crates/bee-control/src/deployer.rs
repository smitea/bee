//! Deployer — the S09 orchestrator.
//!
//! Takes a `Pipeline` (DAG of TaskSpecs + edges), computes a placement plan
//! (round-robin across workers for now — replaced by the real Scheduler in
//! S10), submits `RegisterJob` / `RegisterTask` to the ControlPlane SM
//! (S08), deploys each Task on its assigned TaskWorker, and wires the
//! cross-worker data channel so Task A on Worker X can emit to Task B on
//! Worker Y.
//!
//! ## Cross-worker data channel (S09 in-process)
//! Each Task's runtime output flows through a per-Task forwarder task
//! that the Deployer spawns. The forwarder reads from the worker's task
//! output, looks up the routing table, and sends to the destination
//! worker's task input. For S09 this is all in-process mpsc. The swap to
//! BRP-over-TCP (bee-transport from S02) is a transport change in the
//! forwarder, not a restructuring.

use std::collections::HashMap;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::builtin_handlers::LogSink;
use crate::kv::TaskStatus;
use crate::kv::{Op, TxnError};
use crate::raft::cluster::{Cluster, ClusterConfig};
use crate::scheduler::{Scheduler, TaskRequirement};
use crate::worker::{TaskWorker, WorkerCapacity, WorkerError};

#[derive(Clone)]
pub struct TaskSpec {
    pub task_id: u32,
    pub phase_id: u32,
    pub handler_kind: HandlerKind,
    pub cpu_millicores: u32,
    pub mem_mb: u32,
}

#[derive(Clone)]
pub enum HandlerKind {
    Started { tag: String },
    Terminal { tag: String },
}

impl HandlerKind {
    pub fn build(&self, log: LogSink) -> Box<dyn bee_runtime::DynHandler> {
        match self {
            HandlerKind::Started { tag } => {
                Box::new(crate::builtin_handlers::StartedHandler::new(tag.clone(), log))
            }
            HandlerKind::Terminal { tag } => {
                Box::new(crate::builtin_handlers::TerminalHandler::new(tag.clone(), log))
            }
        }
    }
}

pub struct Edge {
    pub from: u32,
    pub to: u32,
}

/// S17 §2: the (datasource_name, adapter_method, stream_topology_args)
/// triple extracted from a Pipeline's SQL by
/// [`bee_dsl_sql::preprocess::extract_stream_identities`]. Re-exported
/// here so callers of `Pipeline` don't need to depend on bee-dsl-sql
/// just to spell the field type.
pub type StreamIdentity = (
    String,
    String,
    std::collections::BTreeMap<String, String>,
);

pub struct Pipeline {
    pub name: String,
    pub tasks: Vec<TaskSpec>,
    pub edges: Vec<Edge>,
    /// S17 §2: stream-producing identities extracted from the
    /// original SQL. Populated by [`Pipeline::from_sql`]; left
    /// empty when the Pipeline is built via the struct literal
    /// (legacy path used by `deploy_pipeline.rs`, `work_stealing.rs`,
    /// etc.). When non-empty, `Deployer::deploy` consults the CP
    /// for each signature and routes the Job as Producer or
    /// Subscriber per ADR-0011.
    pub stream_identities: Vec<StreamIdentity>,
}

impl Pipeline {
    pub fn linear_3() -> (Pipeline, LogSink) {
        let log = LogSink::new();
        let p = Pipeline {
            name: "linear-3".to_string(),
            tasks: vec![
                TaskSpec {
                    task_id: 1,
                    phase_id: 0,
                    handler_kind: HandlerKind::Started { tag: "A".to_string() },
                    cpu_millicores: 0,
                    mem_mb: 0,
                },
                TaskSpec {
                    task_id: 2,
                    phase_id: 0,
                    handler_kind: HandlerKind::Started { tag: "B".to_string() },
                    cpu_millicores: 0,
                    mem_mb: 0,
                },
                TaskSpec {
                    task_id: 3,
                    phase_id: 0,
                    handler_kind: HandlerKind::Terminal { tag: "C".to_string() },
                    cpu_millicores: 0,
                    mem_mb: 0,
                },
            ],
            edges: vec![Edge { from: 1, to: 2 }, Edge { from: 2, to: 3 }],
            stream_identities: vec![],
        };
        (p, log)
    }

    /// S17 §2: build a Pipeline from raw SQL. Extracts the
    /// stream identities via [`bee_dsl_sql::preprocess::extract_stream_identities`];
    /// for the MVP, synthesizes a minimal linear DAG (one
    /// `Started` Phase per stream identity, plus a terminal sink).
    /// Future: route through `bee_dsl_sql::compile_to_dag` for a
    /// proper Logical-to-Physical compilation.
    ///
    /// The synthesized DAG has zero resource requirements
    /// (`cpu_millicores = 0`, `mem_mb = 0`) so it always fits on
    /// the smallest worker. Pipelines built via `from_sql` exist
    /// primarily to exercise the deployer's signature-driven
    /// Producer/Subscriber wiring; the runtime DAG shape is a
    /// stand-in until S20+ replaces it with the full compile path.
    pub fn from_sql(
        name: &str,
        sql: &str,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        use bee_dsl_sql::preprocess::extract_stream_identities;
        let stream_identities = extract_stream_identities(sql);
        let mut tasks: Vec<TaskSpec> = stream_identities
            .iter()
            .enumerate()
            .map(|(i, _)| TaskSpec {
                task_id: (i + 1) as u32,
                phase_id: i as u32,
                handler_kind: HandlerKind::Started { tag: format!("P{i}") },
                cpu_millicores: 0,
                mem_mb: 0,
            })
            .collect();
        let sink_id = (tasks.len() + 1) as u32;
        tasks.push(TaskSpec {
            task_id: sink_id,
            phase_id: tasks.len() as u32,
            handler_kind: HandlerKind::Terminal { tag: "SINK".to_string() },
            cpu_millicores: 0,
            mem_mb: 0,
        });
        let mut edges: Vec<Edge> = (1..sink_id)
            .map(|i| Edge { from: i, to: i + 1 })
            .collect();
        if stream_identities.is_empty() {
            // No stream identities = no upstream tasks; drop the
            // synthesized SINK so the pipeline is genuinely empty.
            tasks.pop();
            edges.clear();
        }
        Ok(Pipeline {
            name: name.to_string(),
            tasks,
            edges,
            stream_identities,
        })
    }
}

pub struct DeployerConfig {
    pub num_workers: usize,
    pub cluster_config: ClusterConfig,
    pub worker_capacity: WorkerCapacity,
}

impl Default for DeployerConfig {
    fn default() -> Self {
        Self {
            num_workers: 3,
            cluster_config: ClusterConfig::default(),
            worker_capacity: WorkerCapacity::default(),
        }
    }
}

pub struct Deployer {
    pub cluster: Cluster,
    pub workers: HashMap<u32, TaskWorker>,
    pub log: LogSink,
    pub scheduler: Box<dyn Scheduler>,
    routing: HashMap<(u32, u32), Vec<(u32, u32)>>,
    inputs: HashMap<(u32, u32), mpsc::Sender<i64>>,
    forwarder_handles: Vec<JoinHandle<()>>,
}

impl Deployer {
    pub async fn new(config: DeployerConfig) -> Self {
        Self::with_scheduler(
            config,
            Box::new(crate::scheduler::FirstFitDecreasingScheduler::new()),
        )
        .await
    }

    pub async fn with_scheduler(
        config: DeployerConfig,
        scheduler: Box<dyn Scheduler>,
    ) -> Self {
        let cluster = Cluster::new(config.cluster_config).await;
        let log = LogSink::new();
        let mut workers = HashMap::new();
        for i in 1..=config.num_workers as u32 {
            workers.insert(
                i,
                TaskWorker::with_capacity(i, config.worker_capacity.clone(), log.clone()),
            );
        }
        Self {
            cluster,
            workers,
            log,
            scheduler,
            routing: HashMap::new(),
            inputs: HashMap::new(),
            forwarder_handles: Vec::new(),
        }
    }

    pub fn worker(&self, node_id: u32) -> Option<&TaskWorker> {
        self.workers.get(&node_id)
    }

    pub fn worker_mut(&mut self, node_id: u32) -> Option<&mut TaskWorker> {
        self.workers.get_mut(&node_id)
    }

    pub fn log(&self) -> &LogSink {
        &self.log
    }

    pub fn log_contains(&self, needle: &str) -> bool {
        self.log.contains(needle)
    }

    pub fn log_messages(&self) -> Vec<String> {
        self.log.messages()
    }

    /// Deploy a Pipeline. Returns the assigned JobId.
    pub async fn deploy(&mut self, pipeline: Pipeline) -> Result<u32, DeployError> {
        // S33-deferred §3: pre-flight check. Every Plugin-kind
        // adapter the Pipeline references must be registered
        // in the PluginAdapterRegistry. The MVP warns on
        // unknown names; a follow-up should make this a hard
        // error.
        let reg = bee_runtime::plugin_adapter::PluginAdapterRegistry::global();
        for ident in &pipeline.stream_identities {
            if reg.lookup_input(&ident.0).is_none() {
                eprintln!(
                    "warning: stream identity {}.{} has no registered input adapter; runtime will fail at Phase start",
                    ident.0, ident.1
                );
            }
        }

        let leader = self
            .cluster
            .wait_for_leader(Duration::from_secs(3))
            .await
            .ok_or(DeployError::NoLeader)?;
        let job_id = next_job_id();

        if self.workers.is_empty() {
            return Err(DeployError::NoWorkers);
        }

        let requirements: Vec<TaskRequirement> = pipeline
            .tasks
            .iter()
            .map(|t| TaskRequirement {
                task_id: t.task_id,
                cpu_millicores: t.cpu_millicores,
                mem_mb: t.mem_mb,
            })
            .collect();
        let mut node_capacities: Vec<crate::scheduler::NodeCapacity> = self
            .workers
            .values()
            .map(|w| crate::scheduler::NodeCapacity {
                node_id: w.node_id,
                cpu_millicores_total: w.capacity.cpu_millicores_total,
                mem_mb_total: w.capacity.mem_mb_total,
            })
            .collect();
        node_capacities.sort_by_key(|n| n.node_id);

        let placements = self.scheduler.place(&requirements, &node_capacities);
        let mut placement_map: HashMap<u32, u32> = HashMap::new();
        for (task, slot) in pipeline.tasks.iter().zip(placements.iter()) {
            match slot {
                Some(p) => {
                    placement_map.insert(task.task_id, p.node_id);
                }
                None => {
                    return Err(DeployError::InsufficientCapacity {
                        task_id: task.task_id,
                    });
                }
            }
        }

            self.cluster
                .submit(
                    leader,
                    Op::RegisterJob {
                        job_id,
                        dag_hash: pipeline.name.clone(),
                        owner_node: leader,
                        tenant: 0,
                        dependencies: vec![],
                        plugins: vec![],
                    },
                )
            .await
            .map_err(DeployError::Submit)?;

        // S17 §2 (ADR-0011): for each stream identity declared by
        // the Pipeline's SQL, decide whether this Job is the
        // Producer (first writer) or a Subscriber (reuses an
        // existing Producer's stream). The decision is made by
        // looking up the leader's CP for the signature; the CP
        // itself is the source of truth and applies the
        // "first writer wins" rule on its own.
        //
        // Pipelines built via the struct literal (legacy path —
        // `deploy_pipeline.rs`, `work_stealing.rs`, etc.) have an
        // empty `stream_identities` vec, so this loop is a no-op
        // for them. Only Pipelines built via `Pipeline::from_sql`
        // exercise the Producer/Subscriber routing.
        for (ds_name, method, args) in &pipeline.stream_identities {
            let sig = crate::signature::stream_signature(ds_name, method, args);
            let existing_producer: Option<u32> = {
                let handle = self
                    .cluster
                    .node(leader)
                    .expect("leader handle exists after wait_for_leader");
                let cp = handle.cp.lock().await;
                cp.lookup_datasource_producer(&sig)
            };
            let s17_op = match existing_producer {
                Some(producer_id) if producer_id != job_id => Op::RegisterDependency {
                    downstream_job: job_id,
                    upstream_job: producer_id,
                    stream: sig,
                },
                _ => Op::RegisterDatasourceProducer {
                    signature: sig,
                    job_id,
                },
            };
            self.cluster
                .submit(leader, s17_op)
                .await
                .map_err(DeployError::Submit)?;
        }

        for task in &pipeline.tasks {
            let worker_id = placement_map[&task.task_id];

            self.cluster
                .submit(
                    leader,
                    Op::RegisterTask {
                        task_id: task.task_id,
                        job_id,
                        phase_id: task.phase_id,
                        owner_node: worker_id,
                        status: TaskStatus::Scheduled,
                        started_at_ms: 0, // MVP: Deployer doesn't track wall-clock; Rebalancer tests inject it
                    },
                )
                .await
                .map_err(DeployError::Submit)?;

            let handler = task.handler_kind.build(self.log.clone());
            let worker = self
                .workers
                .get_mut(&worker_id)
                .ok_or(DeployError::NoWorker(worker_id))?;
            worker
                .deploy_dyn(task.task_id, handler)
                .map_err(DeployError::Worker)?;
        }

        for edge in &pipeline.edges {
            let from = (placement_map[&edge.from], edge.from);
            let to = (placement_map[&edge.to], edge.to);
            self.routing.entry(from).or_default().push(to);
        }

        for ((worker_id, task_id), dests) in self.routing.clone() {
            let worker = self
                .workers
                .get_mut(&worker_id)
                .ok_or(DeployError::NoWorker(worker_id))?;
            let mut output_rx = worker
                .take_output(task_id)
                .ok_or(DeployError::NoTask(task_id))?;

            // Take the input channels of all destinations and store them
            // in self.inputs so forwarders can use them.
            let dest_inputs: HashMap<(u32, u32), mpsc::Sender<i64>> = dests
                .iter()
                .map(|&(w, t)| {
                    let worker = self.workers.get(&w).expect("worker exists");
                    let input_tx = worker
                        .input_sender(t)
                        .expect("task input exists");
                    ((w, t), input_tx)
                })
                .collect();
            for (k, v) in &dest_inputs {
                self.inputs.insert(*k, v.clone());
            }

            let routing = self.routing.clone();
            let inputs = self.inputs.clone();
            let log = self.log.clone();
            let from = (worker_id, task_id);
            let handle = tokio::spawn(async move {
                while let Some(data) = output_rx.recv().await {
                    if let Some(dests) = routing.get(&from) {
                        for &to in dests {
                            if let Some(tx) = inputs.get(&to) {
                                log.record(format!(
                                    "forward: {:?} -> {:?} data={}",
                                    from, to, data
                                ));
                                let _ = tx.send(data).await;
                            }
                        }
                    }
                }
            });
            self.forwarder_handles.push(handle);
        }

        for task in &pipeline.tasks {
            self.cluster
                .submit(
                    leader,
                    Op::UpdateTaskStatus {
                        task_id: task.task_id,
                        new_status: TaskStatus::Running,
                    },
                )
                .await
                .map_err(DeployError::Submit)?;
        }

        Ok(job_id)
    }

    pub async fn wait_for_terminal_receive(
        &self,
        _worker_id: u32,
        task_id: u32,
        expected: i64,
        timeout: Duration,
    ) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if self
                .log
                .contains(&format!("received {}", expected))
            {
                return true;
            }
            if self
                .log
                .contains(&format!("T{task_id}: started (input={expected})"))
            {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    pub async fn wait_for_log(&self, needle: &str, timeout: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if self.log.contains(needle) {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    pub async fn job_to_node_mapping(&self, job_id: u32) -> HashMap<u32, u32> {
        let mut out = HashMap::new();
        if let Some((_, handle)) = self.cluster.nodes().next() {
            let cp = &handle.cp;
            let cp = cp.lock().await;
            for task in cp.list_tasks() {
                if task.job_id == job_id {
                    out.insert(task.task_id, task.owner_node);
                }
            }
        }
        out
    }
}

pub fn next_job_id() -> u32 {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(1);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[derive(Debug)]
pub enum DeployError {
    NoLeader,
    NoWorkers,
    NoWorker(u32),
    NoTask(u32),
    InsufficientCapacity { task_id: u32 },
    Submit(TxnError),
    Worker(WorkerError),
}

impl std::fmt::Display for DeployError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeployError::NoLeader => write!(f, "no leader elected"),
            DeployError::NoWorkers => write!(f, "no workers available"),
            DeployError::NoWorker(id) => write!(f, "no worker with id {id}"),
            DeployError::NoTask(id) => write!(f, "no task with id {id}"),
            DeployError::InsufficientCapacity { task_id } => {
                write!(f, "insufficient capacity to place task {task_id}")
            }
            DeployError::Submit(e) => write!(f, "submit failed: {e:?}"),
            DeployError::Worker(e) => write!(f, "worker error: {e:?}"),
        }
    }
}

impl std::error::Error for DeployError {}
