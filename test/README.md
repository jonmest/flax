# Flax Load Balancer - Testing & Benchmarking

This directory contains a complete testing setup for benchmarking the Flax load balancer against direct backend access.

## Overview

The test setup includes:
- **3 nginx backends** running on ports 8081, 8082, 8083
- **Benchmark script** to measure performance and overhead
- **Distribution testing** to verify round-robin selection

## Quick Start

### 1. Start the Backend Servers

```bash
cd test
docker-compose up -d
```

This will start 3 nginx instances:
- Backend 1: http://localhost:8081
- Backend 2: http://localhost:8082
- Backend 3: http://localhost:8083

Verify backends are running:
```bash
curl http://localhost:8081/health
curl http://localhost:8082/health
curl http://localhost:8083/health
```

### 2. Build and Run Flax Load Balancer

In the project root:
```bash
cargo build --release
./target/release/flax
```

The load balancer will listen on http://localhost:3000 and distribute requests across all three backends.

### 3. Run Benchmarks

```bash
cd test
./benchmark.sh
```

The script will:
1. Test direct backend performance (baseline)
2. Test load balancer performance
3. Verify round-robin distribution
4. Calculate overhead

## Benchmark Requirements

Install either `wrk` (recommended) or Apache Bench (`ab`):

**wrk (recommended):**
```bash
# Ubuntu/Debian
sudo apt-get install wrk

# macOS
brew install wrk
```

**Apache Bench (fallback):**
```bash
# Ubuntu/Debian
sudo apt-get install apache2-utils

# macOS
brew install httpd
```

## Manual Testing

### Test Direct Backend Access

```bash
# Single request
curl -i http://localhost:8081/small.txt

# Check which backend served the request
curl -I http://localhost:8081/ | grep X-Backend-ID
```

### Test Through Load Balancer

```bash
# Single request
curl -i http://localhost:3000/small.txt

# Multiple requests to see round-robin
for i in {1..9}; do
  curl -s -I http://localhost:3000/ | grep X-Backend-ID
done
```

### Load Testing with wrk

**Direct to backend:**
```bash
wrk -t4 -c100 -d30s http://localhost:8081/small.txt
```

**Through load balancer:**
```bash
wrk -t4 -c100 -d30s http://localhost:3000/small.txt
```

**High concurrency test:**
```bash
wrk -t8 -c1000 -d30s http://localhost:3000/small.txt
```

### Load Testing with Apache Bench

```bash
# Direct to backend
ab -n 100000 -c 100 http://localhost:8081/small.txt

# Through load balancer
ab -n 100000 -c 100 http://localhost:3000/small.txt
```

## Test Files

- **`/small.txt`** - Tiny file (62 bytes) for minimal overhead testing
- **`/index.html`** - Larger HTML file (~1KB) for realistic testing
- **`/health`** - Health check endpoint (nginx only)

## Expected Results

### Baseline (Direct nginx)
On modern hardware, you should see:
- **Throughput:** 50,000+ req/s
- **Latency:** p99 < 2ms

### Through Flax Load Balancer
Target performance:
- **Throughput:** Similar to direct (within 5-10%)
- **Latency:** p99 < 3ms (< 1ms overhead)
- **Distribution:** Even distribution across 3 backends

### What to Look For

✅ **Good signs:**
- Minimal throughput degradation (< 10%)
- Low latency overhead (< 1ms p99)
- Even backend distribution (33% ± 2% each)
- No errors under load

⚠️ **Issues to investigate:**
- Throughput drop > 20%
- Latency spikes
- Uneven distribution
- Connection errors

## Troubleshooting

### Backends not responding
```bash
# Check containers are running
docker-compose ps

# View logs
docker-compose logs backend1

# Restart
docker-compose restart
```

### Port already in use
```bash
# Check what's using port 3000
sudo lsof -i :3000

# Or ports 8081-8083
sudo lsof -i :8081
```

### Low performance
- Ensure release build: `cargo build --release`
- Check CPU affinity is working (should see messages about core pinning)
- Increase ulimit if needed: `ulimit -n 65536`

## Cleanup

```bash
# Stop backends
docker-compose down

# Stop load balancer
# Ctrl+C in the terminal running flax
```

## Advanced Testing

### Custom Benchmark Parameters

Edit variables in `benchmark.sh`:
```bash
DURATION=30        # Test duration in seconds
THREADS=8          # Number of threads
CONNECTIONS=500    # Concurrent connections
URL_PATH="/index.html"  # File to request
```

### Testing with Different Payload Sizes

Create test files:
```bash
# 1KB file
dd if=/dev/zero of=www/1kb.bin bs=1024 count=1

# 10KB file
dd if=/dev/zero of=www/10kb.bin bs=1024 count=10

# Test
wrk -t4 -c100 -d10s http://localhost:3000/10kb.bin
```

### Stress Testing

Test with many connections:
```bash
wrk -t16 -c2000 -d60s --latency http://localhost:3000/small.txt
```

Monitor system resources:
```bash
# In another terminal
htop
# or
top
```

## Performance Tips

1. **Build with optimizations:**
   ```bash
   RUSTFLAGS="-C target-cpu=native" cargo build --release
   ```

2. **Increase system limits:**
   ```bash
   # Temporary
   ulimit -n 65536

   # Permanent (add to /etc/security/limits.conf)
   * soft nofile 65536
   * hard nofile 65536
   ```

3. **Disable CPU frequency scaling:**
   ```bash
   # Set performance mode
   sudo cpupower frequency-set -g performance
   ```

## Metrics to Track

When benchmarking, pay attention to:

- **Requests/sec** - Raw throughput
- **Transfer/sec** - Bandwidth utilization
- **Latency distribution** - p50, p95, p99, max
- **CPU usage** - Per-core utilization
- **Memory usage** - Should be stable, no leaks
- **Error rate** - Should be 0%

## Comparison with Other Load Balancers

To compare with HAProxy or nginx:

```bash
# HAProxy example (you'd need to set up haproxy.cfg)
wrk -t4 -c100 -d10s http://localhost:8080/small.txt

# Compare results side by side
```

## Contributing Test Cases

To add new tests:
1. Add test files to `www/`
2. Update `benchmark.sh` if needed
3. Document expected results
