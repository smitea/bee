//! `bee-runtime` — Bee 数据面执行单元。
//!
//! 定义 [`Phase`] / [`Handler`] trait / [`Dag`] / [`Runtime`],驱动单个 Task
//! 的输入-处理-输出循环与生命周期 (含 Checkpoint / Migrating)。
//!
//! ## 阶段路线
//! - S03: 1-Phase DAG + [`Handler`] trait + [`PassthroughHandler`] 夹具
//! - S04: [`Runtime`] 拉起 2-Phase DAG,Phase 间用 `tokio::sync::mpsc` 直连
//! - S05 (当前): 多 Phase DAG + 拓扑序 + 分叉 (fan-out / fan-in) + cycle detection
//! - S10: 接入端到端 Pipeline 执行

use std::any::Any;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub mod test_utils;

pub type PhaseId = u32;

#[derive(Clone)]
pub struct Msg(Arc<Box<dyn Any + Send + Sync>>);

impl Msg {
    pub fn new<T: Any + Send + Sync>(v: T) -> Self {
        Self(Arc::new(Box::new(v)))
    }

    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        (**self.0).downcast_ref::<T>()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AdapterRef(pub u32);

#[derive(Debug)]
pub enum RuntimeError {
    Handler(String),
    TypeMismatch { phase: PhaseId },
    ChannelClosed,
    ChannelSend,
    Join,
    Topology(String),
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RuntimeError::Handler(s) => write!(f, "handler error: {s}"),
            RuntimeError::TypeMismatch { phase } => {
                write!(f, "type mismatch at phase {phase}")
            }
            RuntimeError::ChannelClosed => write!(f, "channel closed"),
            RuntimeError::ChannelSend => write!(f, "channel send failed"),
            RuntimeError::Join => write!(f, "task join failed"),
            RuntimeError::Topology(s) => write!(f, "topology error: {s}"),
        }
    }
}

impl std::error::Error for RuntimeError {}

pub trait Handler: Send + 'static {
    type Input: Send + 'static;
    type Output: Send + Sync + 'static;

    fn handle(
        &mut self,
        input: Self::Input,
    ) -> impl Future<Output = Result<Option<Self::Output>, RuntimeError>> + Send;

    fn finish(self) -> impl Future<Output = Result<(), RuntimeError>> + Send
    where
        Self: Sized;
}

pub struct Phase<H: Handler> {
    pub id: PhaseId,
    pub name: String,
    pub adapter: Option<AdapterRef>,
    pub handler: H,
}

impl<H: Handler> Phase<H> {
    pub fn new(id: PhaseId, name: impl Into<String>, handler: H) -> Self {
        Self {
            id,
            name: name.into(),
            adapter: None,
            handler,
        }
    }

    pub fn with_adapter(mut self, adapter: AdapterRef) -> Self {
        self.adapter = Some(adapter);
        self
    }

    pub async fn run(
        &mut self,
        input: H::Input,
    ) -> Result<Option<H::Output>, RuntimeError> {
        self.handler.handle(input).await
    }

    pub async fn finish(self) -> Result<(), RuntimeError> {
        self.handler.finish().await
    }
}

pub trait DynHandler: Send + 'static {
    fn handle_boxed<'a>(
        &'a mut self,
        input: Msg,
    ) -> Pin<
        Box<dyn Future<Output = Result<Option<Msg>, RuntimeError>> + Send + 'a>,
    >;

    fn finish_boxed<'a>(
        self: Box<Self>,
    ) -> Pin<Box<dyn Future<Output = Result<(), RuntimeError>> + Send + 'a>>;
}

impl<H> DynHandler for H
where
    H: Handler,
    H::Input: Clone + 'static,
{
    fn handle_boxed<'a>(
        &'a mut self,
        input: Msg,
    ) -> Pin<
        Box<dyn Future<Output = Result<Option<Msg>, RuntimeError>> + Send + 'a>,
    > {
        Box::pin(async move {
            let arc: Arc<Box<dyn Any + Send + Sync>> = input.0;
            let typed: H::Input = (**arc)
                .downcast_ref::<H::Input>()
                .ok_or(RuntimeError::TypeMismatch { phase: 0 })?
                .clone();
            let output_opt = Handler::handle(self, typed).await?;
            Ok(output_opt.map(|o| Msg::new(o)))
        })
    }

    fn finish_boxed<'a>(
        self: Box<Self>,
    ) -> Pin<Box<dyn Future<Output = Result<(), RuntimeError>> + Send + 'a>> {
        Box::pin(async move { Handler::finish(*self).await })
    }
}

pub struct DynPhase {
    pub id: PhaseId,
    pub name: String,
    pub adapter: Option<AdapterRef>,
    pub output_schema: Option<Arc<arrow_schema::Schema>>,
    pub handler: Box<dyn DynHandler>,
}

impl DynPhase {
    pub fn new<H: Handler>(id: PhaseId, name: impl Into<String>, handler: H) -> Self
    where
        H: 'static,
        H::Input: Clone + 'static,
    {
        Self {
            id,
            name: name.into(),
            adapter: None,
            output_schema: None,
            handler: Box::new(handler),
        }
    }

    pub fn with_adapter(mut self, adapter: AdapterRef) -> Self {
        self.adapter = Some(adapter);
        self
    }

    pub fn with_output_schema(mut self, schema: Arc<arrow_schema::Schema>) -> Self {
        self.output_schema = Some(schema);
        self
    }

    pub fn output_schema(&self) -> Option<&Arc<arrow_schema::Schema>> {
        self.output_schema.as_ref()
    }
}

pub struct Dag {
    vertices: Vec<DynPhase>,
    edges: Vec<(PhaseId, PhaseId)>,
}

impl Dag {
    pub fn new() -> Self {
        Self {
            vertices: Vec::new(),
            edges: Vec::new(),
        }
    }

    pub fn add_phase(&mut self, phase: DynPhase) -> &mut Self {
        self.vertices.push(phase);
        self
    }

    pub fn add_edge(&mut self, from: PhaseId, to: PhaseId) -> Result<&mut Self, RuntimeError> {
        if from == to {
            return Err(RuntimeError::Topology(format!(
                "self-loop on phase {from} is a cycle"
            )));
        }
        if !self.vertex_ids().contains(&from) || !self.vertex_ids().contains(&to) {
            return Err(RuntimeError::Topology(format!(
                "edge {from} -> {to} references unknown phase"
            )));
        }
        if self.is_reachable(to, from) {
            return Err(RuntimeError::Topology(format!(
                "edge {from} -> {to} would create a cycle"
            )));
        }
        self.edges.push((from, to));
        Ok(self)
    }

    pub fn vertices(&self) -> &[DynPhase] {
        &self.vertices
    }

    pub fn edges(&self) -> &[(PhaseId, PhaseId)] {
        &self.edges
    }

    pub fn vertex_ids(&self) -> HashSet<PhaseId> {
        self.vertices.iter().map(|p| p.id).collect()
    }

    fn is_reachable(&self, start: PhaseId, target: PhaseId) -> bool {
        let adj: BTreeMap<PhaseId, Vec<PhaseId>> =
            self.edges
                .iter()
                .fold(BTreeMap::new(), |mut m, &(f, t)| {
                    m.entry(f).or_default().push(t);
                    m
                });
        let mut visited = HashSet::new();
        let mut stack = vec![start];
        while let Some(node) = stack.pop() {
            if node == target {
                return true;
            }
            if !visited.insert(node) {
                continue;
            }
            if let Some(neighbors) = adj.get(&node) {
                for &n in neighbors {
                    stack.push(n);
                }
            }
        }
        false
    }
}

impl Default for Dag {
    fn default() -> Self {
        Self::new()
    }
}

pub struct PassthroughHandler;

impl Handler for PassthroughHandler {
    type Input = Vec<u8>;
    type Output = Vec<u8>;

    fn handle(
        &mut self,
        input: Vec<u8>,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, RuntimeError>> + Send {
        async move { Ok(Some(input)) }
    }

    fn finish(self) -> impl Future<Output = Result<(), RuntimeError>> + Send {
        async move { Ok(()) }
    }
}

pub struct MapHandler<F, T> {
    f: F,
    _phantom: std::marker::PhantomData<T>,
}

impl<F, T> MapHandler<F, T> {
    pub fn new(f: F) -> Self {
        Self {
            f,
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<F, T> Handler for MapHandler<F, T>
where
    F: Fn(T) -> T + Send + Sync + 'static,
    T: Send + Sync + 'static,
{
    type Input = T;
    type Output = T;

    fn handle(
        &mut self,
        input: T,
    ) -> impl Future<Output = Result<Option<T>, RuntimeError>> + Send {
        let f = &self.f;
        async move { Ok(Some(f(input))) }
    }

    fn finish(self) -> impl Future<Output = Result<(), RuntimeError>> + Send {
        async move { Ok(()) }
    }
}

pub struct FilterHandler<F, T> {
    f: F,
    _phantom: std::marker::PhantomData<T>,
}

impl<F, T> FilterHandler<F, T> {
    pub fn new(f: F) -> Self {
        Self {
            f,
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<F, T> Handler for FilterHandler<F, T>
where
    F: Fn(&T) -> bool + Send + Sync + 'static,
    T: Send + Sync + 'static,
{
    type Input = T;
    type Output = T;

    fn handle(
        &mut self,
        input: T,
    ) -> impl Future<Output = Result<Option<T>, RuntimeError>> + Send {
        let f = &self.f;
        async move {
            if f(&input) {
                Ok(Some(input))
            } else {
                Ok(None)
            }
        }
    }

    fn finish(self) -> impl Future<Output = Result<(), RuntimeError>> + Send {
        async move { Ok(()) }
    }
}

pub struct Runtime;

impl Runtime {
    pub fn run(
        dag: Dag,
        input_rx: mpsc::Receiver<Msg>,
        output_tx: mpsc::Sender<Msg>,
    ) -> JoinHandle<Result<(), RuntimeError>> {
        tokio::spawn(async move { run_inner(dag, input_rx, output_tx).await })
    }
}

async fn run_inner(
    dag: Dag,
    input_rx: mpsc::Receiver<Msg>,
    output_tx: mpsc::Sender<Msg>,
) -> Result<(), RuntimeError> {
    let Dag { vertices, edges } = dag;

    if vertices.is_empty() {
        return Err(RuntimeError::Topology("empty dag".to_string()));
    }

    let mut phases: HashMap<PhaseId, DynPhase> =
        vertices.into_iter().map(|p| (p.id, p)).collect();

    let mut incoming: HashMap<PhaseId, Vec<PhaseId>> = HashMap::new();
    let mut outgoing: HashMap<PhaseId, Vec<PhaseId>> = HashMap::new();
    for &(from, to) in &edges {
        incoming.entry(to).or_default().push(from);
        outgoing.entry(from).or_default().push(to);
    }

    let source_ids: Vec<PhaseId> = phases
        .keys()
        .filter(|id| !incoming.contains_key(id))
        .copied()
        .collect();
    let sink_ids: Vec<PhaseId> = phases
        .keys()
        .filter(|id| !outgoing.contains_key(id))
        .copied()
        .collect();

    if source_ids.len() != 1 {
        return Err(RuntimeError::Topology(format!(
            "runtime requires exactly 1 source, found {}",
            source_ids.len()
        )));
    }
    if sink_ids.len() != 1 {
        return Err(RuntimeError::Topology(format!(
            "runtime requires exactly 1 sink, found {}",
            sink_ids.len()
        )));
    }
    let source_id = source_ids[0];
    let sink_id = sink_ids[0];

    let topo_order = topological_sort(&phases.keys().copied().collect::<Vec<_>>(), &edges)?;

    let mut phase_inputs: HashMap<PhaseId, mpsc::Receiver<Msg>> = HashMap::new();
    let mut phase_input_senders: HashMap<PhaseId, mpsc::Sender<Msg>> = HashMap::new();
    let mut phase_outputs: HashMap<PhaseId, Vec<mpsc::Sender<Msg>>> = HashMap::new();

    for &id in phases.keys() {
        phase_outputs.insert(id, Vec::new());
    }

    for &(from, to) in &edges {
        let tx = if let Some(existing) = phase_input_senders.get(&to) {
            existing.clone()
        } else {
            let (tx, rx) = mpsc::channel::<Msg>(16);
            phase_input_senders.insert(to, tx.clone());
            phase_inputs.insert(to, rx);
            tx
        };
        phase_outputs.get_mut(&from).unwrap().push(tx);
    }

    drop(phase_input_senders);

    phase_inputs.insert(source_id, input_rx);
    phase_outputs.insert(sink_id, vec![output_tx]);

    let mut handles: Vec<JoinHandle<Result<(), RuntimeError>>> = Vec::new();

    for id in topo_order {
        let phase = phases
            .remove(&id)
            .ok_or(RuntimeError::Topology(format!("phase {id} missing")))?;
        let input_rx = phase_inputs.remove(&id).ok_or(RuntimeError::Topology(format!(
            "phase {id} has no input channel"
        )))?;
        let output_txs = phase_outputs.remove(&id).unwrap_or_default();

        let _ = id;
        let _ = source_id;
        let _ = sink_id;
        handles.push(tokio::spawn(async move {
            run_phase_loop(phase, input_rx, output_txs).await
        }));
    }

    for h in handles {
        h.await.map_err(|_| RuntimeError::Join)??;
    }
    Ok(())
}

async fn run_phase_loop(
    mut phase: DynPhase,
    mut input_rx: mpsc::Receiver<Msg>,
    output_txs: Vec<mpsc::Sender<Msg>>,
) -> Result<(), RuntimeError> {
    while let Some(input) = input_rx.recv().await {
        let output = phase.handler.handle_boxed(input).await?;
        if let Some(out) = output {
            for tx in &output_txs {
                tx.send(out.clone())
                    .await
                    .map_err(|_| RuntimeError::ChannelSend)?;
            }
        }
    }
    let DynPhase { handler, .. } = phase;
    handler.finish_boxed().await?;
    Ok(())
}

fn topological_sort(
    vertices: &[PhaseId],
    edges: &[(PhaseId, PhaseId)],
) -> Result<Vec<PhaseId>, RuntimeError> {
    let mut in_degree: HashMap<PhaseId, usize> =
        vertices.iter().map(|&id| (id, 0)).collect();
    let mut out: BTreeMap<PhaseId, Vec<PhaseId>> = BTreeMap::new();
    for &(from, to) in edges {
        *in_degree.entry(to).or_insert(0) += 1;
        out.entry(from).or_default().push(to);
    }

    let mut queue: Vec<PhaseId> = in_degree
        .iter()
        .filter(|(_, &d)| d == 0)
        .map(|(id, _)| *id)
        .collect();
    let mut sorted = Vec::new();
    while let Some(id) = queue.pop() {
        sorted.push(id);
        if let Some(neighbors) = out.get(&id) {
            for &n in neighbors {
                if let Some(d) = in_degree.get_mut(&n) {
                    *d -= 1;
                    if *d == 0 {
                        queue.push(n);
                    }
                }
            }
        }
    }
    if sorted.len() != vertices.len() {
        return Err(RuntimeError::Topology(
            "cycle detected in dag (topological sort failed)".to_string(),
        ));
    }
    Ok(sorted)
}
