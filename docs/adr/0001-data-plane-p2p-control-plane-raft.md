# 0001: Data Plane P2P + Control Plane Raft

The original sketch labeled Bee a "去中心化网络" while also describing a Raft Leader arbitrating StealTask — these are mutually exclusive. We resolve the contradiction by adopting a hybrid architecture that matches the way the rest of the design is already written: BRP carries P2P data flow between Nodes, while a Raft-replicated Control Plane owns all "who owns what" state (membership, Pipeline/Phase/Handler/Datasource ownership, orphan detection, Work-Stealing arbitration). The data path never touches the Raft Leader; the control path always does.

## Consequences

- The Data Plane must tolerate the Control Plane being temporarily unavailable (e.g., Leader election in progress) — it should keep flowing buffered data, not block.
- Every "who owns this Phase" lookup is a Raft read or local cache; we must define staleness bounds for the cache later.
- Two channels over BRP: a low-latency data channel and a higher-latency control RPC channel.
