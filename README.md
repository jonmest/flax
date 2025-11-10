# Flax - High-Performance Load Balancer Framework

A high-performance, io_uring-based load balancer framework written in Rust, designed for maximum throughput and minimal latency.

## Vision

Flax is a framework for building production-grade load balancers that can handle millions of connections with microsecond latencies. It leverages Linux io_uring for zero-copy I/O operations and provides a modular architecture for building custom load balancing solutions.

## Architecture

Flax is organized into distinct layers, each with specific responsibilities:

### Layer 1: Core (`src/core/`)
Low-level networking primitives and io_uring abstractions.

**Components:**
- **`socket.rs`** - Socket creation, configuration, and management
- **`connection_pair.rs`** - Client-backend connection state management
- **`buffer.rs`** - Zero-copy buffer management and memory pools
- **`stream_pump.rs`** - Bidirectional data streaming primitives
- **`user_data.rs`** - io_uring operation tagging and identification
- **`constants.rs`** - System-wide constants and configuration

**Design Goals:**
- Reusable for any io_uring-based application
- Zero-copy where possible
- Minimal allocations in hot path
- CPU cache-friendly data structures

### Layer 2: Protocol (`src/protocol/`)
Protocol-specific parsing and handling.

**Components:**
- **`http1.rs`** - HTTP/1.1 implementation (header parsing, request/response handling)
- **`tcp.rs`** - TCP passthrough mode (L4 load balancing)
- **`http2.rs`** - HTTP/2 support (future)
- **`traits.rs`** - Common protocol abstractions

**Current Status:** Basic HTTP/1.1 parsing exists in `core/http.rs` and `core/http_meta.rs` (to be moved)

### Layer 3: Backend (`src/backend/`)
Backend pool management and selection strategies.

**Components:**
- **`pool.rs`** - Backend pool management (add/remove/list backends)
- **`upstream.rs`** - Individual backend representation with metrics
- **`selector.rs`** - Selection strategies:
  - Round-robin (implemented)
  - Least connections
  - Consistent hashing
  - Weighted round-robin
  - Random
- **`health.rs`** - Active and passive health checking
- **`circuit_breaker.rs`** - Failure detection and circuit breaking

**Current Status:** Basic round-robin pool exists in `core/route.rs` (to be moved and expanded)

### Layer 4: Balancer (`src/balancer/`)
Load balancing logic and request processing pipeline.

**Components:**
- **`pipeline.rs`** - Request processing pipeline
- **`session.rs`** - Session affinity / sticky sessions
- **`retry.rs`** - Retry logic with backoff
- **`timeout.rs`** - Request timeout handling
- **`connection_pool.rs`** - Backend connection pooling and reuse

**Current Status:** Core logic exists in `main.rs::run_worker()` (to be extracted)

### Layer 5: Observability (`src/observability/`)
Metrics, tracing, and monitoring.

**Components:**
- **`metrics.rs`** - Performance metrics:
  - Requests per second
  - Latency histograms (p50, p95, p99)
  - Error rates
  - Active connections
  - Backend health status
- **`tracing.rs`** - Distributed tracing integration
- **`healthz.rs`** - Health check endpoint for the LB itself

**Current Status:** Not yet implemented

### Layer 6: Config (`src/config/`)
Configuration management with hot-reload support.

**Components:**
- **`loader.rs`** - Load config from files, env vars, or API
- **`types.rs`** - Configuration schema and types
- **`validation.rs`** - Configuration validation
- **`hot_reload.rs`** - Watch for config changes and reload

**Current Status:** Not yet implemented

### Layer 7: Control (`src/control/`)
Control plane API for runtime management.

**Components:**
- **`api.rs`** - HTTP/gRPC API for management operations
- **`admin.rs`** - Admin operations (add/remove backends, drain, etc.)
- **`stats.rs`** - Statistics and debug endpoints

**Current Status:** Not yet implemented

### Layer 8: Gossip (`src/gossip/`)
Distributed coordination for multi-node deployments (future).

**Components:**
- **`membership.rs`** - Cluster membership (SWIM protocol)
- **`state_sync.rs`** - Distributed state synchronization
- **`discovery.rs`** - Service discovery integration
- **`consensus.rs`** - Leader election if needed

**Current Status:** Future work

## Current Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                         Client                               │
└────────────────────────┬────────────────────────────────────┘
                         │
                         ▼
         ┌───────────────────────────────┐
         │   SO_REUSEPORT Listeners      │
         │   (one per CPU core)          │
         └───────────────┬───────────────┘
                         │
         ┌───────────────▼───────────────┐
         │   io_uring Workers            │
         │   (pinned to cores)           │
         │                               │
         │  • Accept connections         │
         │  • Parse HTTP headers         │
         │  • Route to backend           │
         │  • Bidirectional streaming    │
         └───────────────┬───────────────┘
                         │
                         ▼
         ┌───────────────────────────────┐
         │   Backend Pool                │
         │   (round-robin selection)     │
         └───────────────┬───────────────┘
                         │
         ┌───────────────▼───────────────┐
         │   Backend Servers             │
         │   (8081, 8082, 8083)          │
         └───────────────────────────────┘
```

## Performance Design

### Per-Core Workers
- Each CPU core runs an independent worker with its own io_uring instance
- SO_REUSEPORT distributes incoming connections across workers
- Workers are pinned to CPU cores using `core_affinity`
- No cross-core communication in the hot path

### Zero-Copy I/O
- io_uring operations avoid system call overhead
- Direct buffer management with minimal allocations
- Connection pairs with bidirectional streaming

### Lock-Free Hot Path
- Atomic counters for round-robin selection
- Read-write locks only for backend pool modifications
- Per-worker connection state (no shared state)

## Roadmap

### Phase 1: Foundation (Current)
- [x] io_uring-based TCP proxy
- [x] HTTP/1.1 header parsing
- [x] Basic round-robin backend selection
- [x] Runtime backend pool management
- [ ] Refactor into layered architecture
- [ ] Basic metrics collection

### Phase 2: Production Features
- [ ] Multiple selection strategies
- [ ] Active health checking
- [ ] Passive health checking (circuit breaker)
- [ ] Connection pooling to backends
- [ ] Request retries
- [ ] Configuration file support
- [ ] Control plane API

### Phase 3: Advanced Features
- [ ] TLS termination
- [ ] HTTP/2 support
- [ ] Session affinity
- [ ] Rate limiting
- [ ] Distributed tracing
- [ ] WebSocket support

### Phase 4: Distributed
- [ ] Gossip protocol for cluster membership
- [ ] Distributed state synchronization
- [ ] Service discovery integration
- [ ] Global load balancing

## Usage

### Basic Example

```rust
use flax::core::route::{init_backend_pool, Backend};

fn main() -> std::io::Result<()> {
    // Initialize backend pool
    init_backend_pool(vec![
        Backend::new("127.0.0.1:8081".parse().unwrap()),
        Backend::new("127.0.0.1:8082".parse().unwrap()),
        Backend::new("127.0.0.1:8083".parse().unwrap()),
    ]);

    // Run load balancer (listens on port 3000)
    flax::run("0.0.0.0:3000".parse().unwrap())
}
```

### Runtime Backend Management

```rust
use flax::core::route::{get_backend_pool, Backend};

// Add a backend
let pool = get_backend_pool();
pool.add_backend(Backend::new("127.0.0.1:8084".parse().unwrap()));

// Remove a backend
pool.remove_backend("127.0.0.1:8081".parse().unwrap());

// List backends
let backends = pool.list_backends();
println!("Active backends: {:?}", backends);
```

## Building

```bash
# Build
cargo build --release

# Run tests
cargo test

# Run benchmarks
cargo bench

# Run examples
cargo run --example manage_backends
```

## Performance Targets

- **Throughput:** 1M+ requests/sec per core
- **Latency:** p99 < 1ms (excluding backend latency)
- **Connections:** 100k+ concurrent connections per instance
- **Memory:** < 1KB per connection

## Design Principles

1. **Zero-cost abstractions** - High-level APIs should compile to optimal code
2. **Framework-first** - Users should be able to build custom LBs with our components
3. **Performance by default** - Sensible defaults for production use
4. **Observable** - Rich metrics and tracing built-in
5. **Composable** - Mix and match components as needed
6. **Safe** - Leverage Rust's type system for correctness

## Contributing

See [ARCHITECTURE.md](ARCHITECTURE.md) for detailed component design.

## License

MIT OR Apache-2.0
