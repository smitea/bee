//! `bee-runtime` — Bee 数据面执行单元。
//!
//! 定义 [`Phase`] / [`Handler`] trait / [`Dag`] / [`Runtime`],驱动单个 Task
//! 的输入-处理-输出循环与生命周期 (含 Checkpoint / Migrating)。
//!
//! ## 阶段路线
//! - S03: 1-Phase DAG + [`Handler`] trait + [`PassthroughHandler`] 夹具
//! - S04 (当前): [`Runtime`] 拉起 2-Phase DAG,Phase 间用 `tokio::sync::mpsc` 直连;
//!   [`MapHandler`] / [`FilterHandler`] 内置处理器
//! - S05: 多 Phase DAG + 拓扑序 + 分叉
//! - S10: 接入端到端 Pipeline 执行

use std::any::Any;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub type PhaseId = u32;

pub type Msg = Box<dyn Any + Send>;

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
    type Output: Send + 'static;

    fn handle(
        &mut self,
        input: Self::Input,
    ) -> impl std::future::Future<Output = Result<Option<Self::Output>, RuntimeError>> + Send;

    fn finish(self) -> impl std::future::Future<Output = Result<(), RuntimeError>> + Send
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

impl<H: Handler> DynHandler for H {
    fn handle_boxed<'a>(
        &'a mut self,
        input: Msg,
    ) -> Pin<
        Box<dyn Future<Output = Result<Option<Msg>, RuntimeError>> + Send + 'a>,
    > {
        Box::pin(async move {
            let input_typed: H::Input = *input
                .downcast::<H::Input>()
                .map_err(|_| RuntimeError::TypeMismatch { phase: 0 })?;
            let output_opt = Handler::handle(self, input_typed).await?;
            Ok(output_opt.map(|o| Box::new(o) as Msg))
        })
    }

    fn finish_boxed<'a>(
        self: Box<Self>,
    ) -> Pin<Box<dyn Future<Output = Result<(), RuntimeError>> + Send + 'a>> {
        Box::pin(async move { (*self).finish().await })
    }
}

pub struct DynPhase {
    pub id: PhaseId,
    pub name: String,
    pub adapter: Option<AdapterRef>,
    pub handler: Box<dyn DynHandler>,
}

impl DynPhase {
    pub fn new<H: Handler>(id: PhaseId, name: impl Into<String>, handler: H) -> Self {
        Self {
            id,
            name: name.into(),
            adapter: None,
            handler: Box::new(handler),
        }
    }

    pub fn with_adapter(mut self, adapter: AdapterRef) -> Self {
        self.adapter = Some(adapter);
        self
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

    pub fn add_edge(&mut self, from: PhaseId, to: PhaseId) -> &mut Self {
        self.edges.push((from, to));
        self
    }

    pub fn vertices(&self) -> &[DynPhase] {
        &self.vertices
    }

    pub fn edges(&self) -> &[(PhaseId, PhaseId)] {
        &self.edges
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
    ) -> impl std::future::Future<Output = Result<Option<Vec<u8>>, RuntimeError>> + Send {
        async move { Ok(Some(input)) }
    }

    fn finish(self) -> impl std::future::Future<Output = Result<(), RuntimeError>> + Send {
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
    T: Send + 'static,
{
    type Input = T;
    type Output = T;

    fn handle(
        &mut self,
        input: T,
    ) -> impl std::future::Future<Output = Result<Option<T>, RuntimeError>> + Send {
        let f = &self.f;
        async move { Ok(Some(f(input))) }
    }

    fn finish(self) -> impl std::future::Future<Output = Result<(), RuntimeError>> + Send {
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
    T: Send + 'static,
{
    type Input = T;
    type Output = T;

    fn handle(
        &mut self,
        input: T,
    ) -> impl std::future::Future<Output = Result<Option<T>, RuntimeError>> + Send {
        let f = &self.f;
        async move {
            if f(&input) {
                Ok(Some(input))
            } else {
                Ok(None)
            }
        }
    }

    fn finish(self) -> impl std::future::Future<Output = Result<(), RuntimeError>> + Send {
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
        return Err(RuntimeError::Topology("dag has no vertices".to_string()));
    }

    let mut phases: HashMap<PhaseId, DynPhase> =
        vertices.into_iter().map(|p| (p.id, p)).collect();

    let mut incoming: HashMap<PhaseId, Vec<PhaseId>> = HashMap::new();
    let mut outgoing: HashMap<PhaseId, Vec<PhaseId>> = HashMap::new();
    for (from, to) in &edges {
        incoming.entry(*to).or_default().push(*from);
        outgoing.entry(*from).or_default().push(*to);
    }

    let mut source_ids: Vec<PhaseId> = phases
        .keys()
        .filter(|id| !incoming.contains_key(id))
        .copied()
        .collect();
    let mut sink_ids: Vec<PhaseId> = phases
        .keys()
        .filter(|id| !outgoing.contains_key(id))
        .copied()
        .collect();

    if source_ids.len() != 1 {
        return Err(RuntimeError::Topology(format!(
            "S04 runtime requires exactly 1 source, found {}",
            source_ids.len()
        )));
    }
    if sink_ids.len() != 1 {
        return Err(RuntimeError::Topology(format!(
            "S04 runtime requires exactly 1 sink, found {}",
            sink_ids.len()
        )));
    }
    let source_id = source_ids.remove(0);
    let sink_id = sink_ids.remove(0);

    let source_phase = phases
        .remove(&source_id)
        .ok_or(RuntimeError::Topology(format!("source phase {source_id} missing")))?;
    let sink_phase = if source_id == sink_id {
        None
    } else {
        Some(
            phases
                .remove(&sink_id)
                .ok_or(RuntimeError::Topology(format!("sink phase {sink_id} missing")))?,
        )
    };

    if !phases.is_empty() {
        return Err(RuntimeError::Topology(
            "S04 runtime supports only 1 or 2 phases".to_string(),
        ));
    }

    if let Some(sink_phase) = sink_phase {
        let (inter_tx, inter_rx) = mpsc::channel::<Msg>(16);
        let source_task = tokio::spawn(async move {
            run_phase_loop(source_phase, input_rx, inter_tx).await
        });
        let sink_task = tokio::spawn(async move {
            run_phase_loop(sink_phase, inter_rx, output_tx).await
        });
        source_task.await.map_err(|_| RuntimeError::Join)??;
        sink_task.await.map_err(|_| RuntimeError::Join)??;
    } else {
        let task = tokio::spawn(async move {
            run_phase_loop(source_phase, input_rx, output_tx).await
        });
        task.await.map_err(|_| RuntimeError::Join)??;
    }

    Ok(())
}

async fn run_phase_loop(
    mut phase: DynPhase,
    mut input_rx: mpsc::Receiver<Msg>,
    output_tx: mpsc::Sender<Msg>,
) -> Result<(), RuntimeError> {
    while let Some(input) = input_rx.recv().await {
        let output = phase.handler.handle_boxed(input).await?;
        if let Some(out) = output {
            output_tx
                .send(out)
                .await
                .map_err(|_| RuntimeError::ChannelSend)?;
        }
    }
    let DynPhase { handler, .. } = phase;
    handler.finish_boxed().await?;
    Ok(())
}
