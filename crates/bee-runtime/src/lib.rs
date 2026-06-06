//! `bee-runtime` — Bee 数据面执行单元。
//!
//! 定义 [`Phase`] / [`Handler`] trait / [`Dag`],驱动单个 Task
//! 的输入-处理-输出循环与生命周期 (含 Checkpoint / Migrating)。
//!
//! ## 阶段路线
//! - S03 (当前): 1-Phase DAG + [`Handler`] trait + [`PassthroughHandler`] 夹具
//! - S04: [`Runtime`] 拉起多 Phase DAG,Phase 间用 `tokio::sync::mpsc` 直连
//! - S05: 多 Phase DAG + 拓扑序 + 分叉
//! - S10: 接入端到端 Pipeline 执行

pub type PhaseId = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AdapterRef(pub u32);

#[derive(Debug)]
pub enum RuntimeError {
    Handler(String),
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RuntimeError::Handler(s) => write!(f, "handler error: {s}"),
        }
    }
}

impl std::error::Error for RuntimeError {}

pub trait Handler: Send + 'static {
    type Input: Send + 'static;
    type Output: Send + 'static;

    #[allow(async_fn_in_trait)]
    async fn handle(&mut self, input: Self::Input) -> Result<Self::Output, RuntimeError>;

    #[allow(async_fn_in_trait)]
    async fn finish(self) -> Result<(), RuntimeError>
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

    pub async fn run(&mut self, input: H::Input) -> Result<H::Output, RuntimeError> {
        self.handler.handle(input).await
    }

    pub async fn finish(self) -> Result<(), RuntimeError> {
        self.handler.finish().await
    }
}

pub struct Dag<H: Handler> {
    phases: Vec<Phase<H>>,
    edges: Vec<(PhaseId, PhaseId)>,
}

impl<H: Handler> Dag<H> {
    pub fn new() -> Self {
        Self {
            phases: Vec::new(),
            edges: Vec::new(),
        }
    }

    pub fn add_phase(&mut self, phase: Phase<H>) -> &mut Self {
        self.phases.push(phase);
        self
    }

    pub fn add_edge(&mut self, from: PhaseId, to: PhaseId) -> &mut Self {
        self.edges.push((from, to));
        self
    }

    pub fn vertices(&self) -> &[Phase<H>] {
        &self.phases
    }

    pub fn edges(&self) -> &[(PhaseId, PhaseId)] {
        &self.edges
    }
}

impl<H: Handler> Default for Dag<H> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct PassthroughHandler;

impl Handler for PassthroughHandler {
    type Input = Vec<u8>;
    type Output = Vec<u8>;

    async fn handle(&mut self, input: Vec<u8>) -> Result<Vec<u8>, RuntimeError> {
        Ok(input)
    }

    async fn finish(self) -> Result<(), RuntimeError> {
        Ok(())
    }
}
