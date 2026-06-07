//! TaskWorker — a single Node's runtime for deployed Tasks.
//!
//! Each TaskWorker wraps a `bee_runtime::Runtime` per deployed Task (1-Phase
//! DAG containing one Handler). The runtime consumes from a per-Task input
//! channel and produces to a per-Task output channel; the Deployer's
//! forwarders wire the output of one Task to the input of the next.
//!
//! For S09 the data channel is `tokio::sync::mpsc` (in-process). The
//! forwarders route between workers on the same process. In a multi-process
//! deployment, the forwarder would send over the BRP data channel
//! (bee-transport from S02) — the worker / deployer API is designed so
//! the swap is a transport-trait change, not a restructuring.

use std::collections::HashMap;

use bee_runtime::{Dag, DynHandler, DynPhase, Msg, Runtime, RuntimeError};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::builtin_handlers::LogSink;

pub struct DeployedTask {
    pub task_id: u32,
    pub input_tx: mpsc::Sender<i64>,
    pub output_rx: Option<mpsc::Receiver<i64>>,
    pub runtime_handle: JoinHandle<Result<Result<(), RuntimeError>, tokio::task::JoinError>>,
}

pub struct TaskWorker {
    pub node_id: u32,
    pub log: LogSink,
    pub deployed: HashMap<u32, DeployedTask>,
}

impl TaskWorker {
    pub fn new(node_id: u32, log: LogSink) -> Self {
        Self {
            node_id,
            log,
            deployed: HashMap::new(),
        }
    }

    pub fn deploy_dyn(
        &mut self,
        task_id: u32,
        handler: Box<dyn DynHandler>,
    ) -> Result<(), WorkerError> {
        let mut dag = Dag::new();
        dag.add_phase(DynPhase {
            id: 0,
            name: "task".to_string(),
            adapter: None,
            handler,
        });
        let (input_tx, input_rx) = mpsc::channel::<Msg>(16);
        let (output_tx, output_rx) = mpsc::channel::<Msg>(16);

        let handle = tokio::spawn(async move { Runtime::run(dag, input_rx, output_tx).await });

        self.deployed.insert(
            task_id,
            DeployedTask {
                task_id,
                input_tx: mpsc_to_i64(input_tx),
                output_rx: Some(mpsc_from_i64(output_rx)),
                runtime_handle: handle,
            },
        );
        Ok(())
    }

    pub fn feed(&self, task_id: u32, data: i64) -> Result<(), WorkerError> {
        let task = self.deployed.get(&task_id).ok_or(WorkerError::NoSuchTask)?;
        task.input_tx
            .try_send(data)
            .map_err(|_| WorkerError::WorkerDead)?;
        Ok(())
    }

    pub fn take_output(&mut self, task_id: u32) -> Option<mpsc::Receiver<i64>> {
        self.deployed
            .get_mut(&task_id)
            .and_then(|t| t.output_rx.take())
    }

    pub fn input_sender(&self, task_id: u32) -> Option<mpsc::Sender<i64>> {
        self.deployed.get(&task_id).map(|t| t.input_tx.clone())
    }

    pub fn log(&self) -> &LogSink {
        &self.log
    }
}

#[derive(Debug)]
pub enum WorkerError {
    NoSuchTask,
    WorkerDead,
}

// Adapters: the runtime uses Msg = Arc<Box<dyn Any + Send + Sync>>;
// the worker / deployer operate in i64 space. The conversions are
// mechanical and isolated to these helpers.

fn mpsc_to_i64(tx: mpsc::Sender<Msg>) -> mpsc::Sender<i64> {
    // We bridge by spawning a forwarder task that translates Msg <-> i64.
    // For now we use a simpler approach: re-create the channel as i64
    // and forward both ways via a small task.
    let (i64_tx, mut i64_rx) = mpsc::channel::<i64>(16);
    tokio::spawn(async move {
        while let Some(i) = i64_rx.recv().await {
            if tx.send(Msg::new(i)).await.is_err() {
                break;
            }
        }
    });
    i64_tx
}

fn mpsc_from_i64(rx: mpsc::Receiver<Msg>) -> mpsc::Receiver<i64> {
    let (i64_tx, i64_rx) = mpsc::channel::<i64>(16);
    let mut rx = rx;
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let val = *msg.downcast_ref::<i64>().unwrap_or(&0);
            if i64_tx.send(val).await.is_err() {
                break;
            }
        }
    });
    i64_rx
}
