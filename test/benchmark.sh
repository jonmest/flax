#!/bin/bash
set -e

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

DURATION=10
THREADS=4
CONNECTIONS=100
URL_PATH="/small.txt"

DIRECT_URL="http://localhost:8081${URL_PATH}"
LB_URL="http://localhost:3000${URL_PATH}"

echo -e "${BLUE}╔════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║    Flax Load Balancer Benchmark Suite     ║${NC}"
echo -e "${BLUE}╔════════════════════════════════════════════╗${NC}"
echo ""

if command -v wrk &> /dev/null; then
    BENCHMARKER="wrk"
    echo -e "${GREEN}✓ Using wrk for benchmarking${NC}"
elif command -v ab &> /dev/null; then
    BENCHMARKER="ab"
    echo -e "${YELLOW}⚠ Using ab (Apache Bench) - wrk recommended for better results${NC}"
else
    echo -e "${RED}✗ Error: Neither wrk nor ab found. Please install one of them:${NC}"
    echo "  Ubuntu/Debian: sudo apt-get install apache2-utils"
    echo "  or install wrk from: https://github.com/wg/wrk"
    exit 1
fi

echo ""
echo -e "${BLUE}Configuration:${NC}"
echo "  Duration:     ${DURATION}s"
echo "  Threads:      ${THREADS}"
echo "  Connections:  ${CONNECTIONS}"
echo "  Test file:    ${URL_PATH}"
echo ""

check_service() {
    local url=$1
    local name=$2
    if curl -s -f -o /dev/null "$url"; then
        echo -e "${GREEN}✓ $name is running${NC}"
        return 0
    else
        echo -e "${RED}✗ $name is not responding${NC}"
        return 1
    fi
}

echo -e "${BLUE}Checking services...${NC}"
check_service "http://localhost:8081/health" "Backend 1 (port 8081)" || exit 1
check_service "http://localhost:8082/health" "Backend 2 (port 8082)" || exit 1
check_service "http://localhost:8083/health" "Backend 3 (port 8083)" || exit 1
echo ""

run_wrk() {
    local url=$1
    local name=$2

    echo -e "${YELLOW}Running benchmark: $name${NC}"
    echo "URL: $url"
    echo ""

    wrk -t${THREADS} -c${CONNECTIONS} -d${DURATION}s "$url"
    echo ""
}

run_ab() {
    local url=$1
    local name=$2
    local total_requests=$((CONNECTIONS * 100))

    echo -e "${YELLOW}Running benchmark: $name${NC}"
    echo "URL: $url"
    echo ""

    ab -n ${total_requests} -c ${CONNECTIONS} -q "$url"
    echo ""
}

echo -e "${BLUE}════════════════════════════════════════════${NC}"
echo -e "${BLUE}  Test 1: Direct Connection to Backend     ${NC}"
echo -e "${BLUE}════════════════════════════════════════════${NC}"
echo ""

if [ "$BENCHMARKER" = "wrk" ]; then
    run_wrk "$DIRECT_URL" "Direct to Backend"
else
    run_ab "$DIRECT_URL" "Direct to Backend"
fi

echo -e "${YELLOW}Is Flax load balancer running on port 3000?${NC}"
echo -e "Start it with: ${GREEN}cargo run --release${NC}"
read -p "Press Enter when ready, or Ctrl+C to skip LB test..."
echo ""

if ! check_service "${LB_URL}" "Flax Load Balancer (port 3000)"; then
    echo -e "${RED}Skipping load balancer test${NC}"
    exit 0
fi
echo ""

echo -e "${BLUE}════════════════════════════════════════════${NC}"
echo -e "${BLUE}  Test 2: Through Flax Load Balancer       ${NC}"
echo -e "${BLUE}════════════════════════════════════════════${NC}"
echo ""

if [ "$BENCHMARKER" = "wrk" ]; then
    run_wrk "$LB_URL" "Through Load Balancer"
else
    run_ab "$LB_URL" "Through Load Balancer"
fi

echo -e "${BLUE}════════════════════════════════════════════${NC}"
echo -e "${BLUE}  Test 3: Backend Distribution Check       ${NC}"
echo -e "${BLUE}════════════════════════════════════════════${NC}"
echo ""

echo "Making 30 requests to check round-robin distribution..."
echo ""

declare -A backend_counts

for i in {1..30}; do
    backend=$(curl -s -H "Connection: close" "${LB_URL}" -D - 2>/dev/null | grep -i "X-Backend-ID" | awk '{print $2}' | tr -d '\r')
    if [ -n "$backend" ]; then
        ((backend_counts[$backend]++)) || backend_counts[$backend]=1
    fi
done

echo "Distribution:"
for backend in "${!backend_counts[@]}"; do
    count=${backend_counts[$backend]}
    echo "  $backend: $count requests"
done

echo ""
echo -e "${GREEN}✓ Benchmark complete!${NC}"
