# Bee Docker Cluster

A 5-node Bee Raft cluster orchestrated via `docker-compose.yml`, with workspace
plugins deployed into per-node volumes by `scripts/deploy-plugins.sh`.

## Layout

```
.
├── Dockerfile                       # Bee node image (builds bee + 3 workspace plugins)
├── Dockerfile.bee-client            # Reference recipe for building the Tauri Bee Client
├── docker-compose.yml               # 5-node cluster (bee-node-1 .. bee-node-5)
├── scripts/deploy-plugins.sh        # Build + deploy plugins into per-node volumes
└── volumes/
    └── node_1/plugins/ .. node_5/plugins/
                                      # Bind-mounted into /etc/bee/plugins per container
```

## Quick start

```bash
# 1. Build the Bee node image and start the 5-node cluster.
docker compose up -d --build

# 2. Compile the three workspace plugins (libbee_plugin_onnx_ml,
#    libbee_plugin_perf_fib, libbee_plugin_sample_kline) and copy them
#    into volumes/node_N/plugins/. If a container is running, the script
#    also `docker cp`s the same .so into the live container.
scripts/deploy-plugins.sh --build --mode docker

# 3. Tail the logs of the cluster leader (or any node).
docker logs -f bee-node-1
```

## Plugin distribution

The deploy script writes the following per-node split (matching the
header comment in `docker-compose.yml`):

| Node  | Bind port | Admin port | Plugins                                                                 |
|-------|-----------|------------|-------------------------------------------------------------------------|
| 1     | 7701      | 8701       | `libbee_plugin_sample_kline`                                            |
| 2     | 7702      | 8702       | `libbee_plugin_perf_fib`                                                |
| 3     | 7703      | 8703       | `libbee_plugin_onnx_ml` (heavy: tract-onnx + FinBERT)                   |
| 4     | 7704      | 8704       | *failover standby* (no plugins)                                         |
| 5     | 7705      | 8705       | `libbee_plugin_onnx_ml`, `libbee_plugin_perf_fib`, `libbee_plugin_sample_kline` (warm pool + work-stealing target) |

Use `--all-to-all` to override the partition and replicate every plugin onto
every node.

## Connecting the Bee Client

After `docker compose up -d`, point the Bee Client (built via
`cargo tauri build` from `app/`) at the leader's admin port on the host loopback:

```
127.0.0.1:8701
```

(The admin ports of every node are exposed on the host as 8701..8705; the
cluster topology tab in the Bee Client will auto-discover all five once it
connects to any one of them.)

## Verifying the cluster

```bash
# Validate the compose file.
docker compose config

# Confirm all five containers are running.
docker compose ps

# Inspect the Raft quorum and commit index from the Bee Client
# (Cluster dashboard → Topology / Raft Leader / Quorum Health / Commit Index).
```

## Bee Client image

`Dockerfile.bee-client` is a reference recipe only — it is not wired into the
cluster compose file because the Tauri bundler produces a host-native installer.
Build locally with:

```bash
cd app
npm ci
npm run tauri build
```

## Cleaning up

```bash
# Stop the cluster and remove containers.
docker compose down

# Wipe built plugin binaries from the per-node volumes (they will be
# rebuilt by the next deploy-plugins.sh run).
rm -rf volumes/node_*/plugins/*.so volumes/node_*/plugins/*.dylib
```