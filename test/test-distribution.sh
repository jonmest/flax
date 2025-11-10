#!/bin/bash

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${BLUE}Testing Round-Robin Distribution${NC}"
echo ""

# Check if LB is running
if ! curl -s -f http://localhost:3000/health > /dev/null 2>&1; then
    if ! curl -s -f http://localhost:3000/small.txt > /dev/null 2>&1; then
        echo -e "${YELLOW}⚠ Load balancer not responding on port 3000${NC}"
        echo "Start it with: cargo run --release"
        exit 1
    fi
fi

echo "Making 100 requests to http://localhost:3000/small.txt"
echo "Collecting backend distribution..."
echo ""

declare -A backend_counts
total=0

for i in {1..100}; do
    # Make request and extract backend ID from header
    backend=$(curl -s -D - http://localhost:3000/small.txt 2>/dev/null | grep -i "X-Backend-ID" | awk '{print $2}' | tr -d '\r')

    if [ -n "$backend" ]; then
        ((backend_counts[$backend]++)) || backend_counts[$backend]=1
        ((total++))
    fi

    # Progress indicator
    if [ $((i % 10)) -eq 0 ]; then
        echo -n "."
    fi
done

echo ""
echo ""
echo -e "${GREEN}Results:${NC}"
echo "────────────────────────────────────"

for backend in "${!backend_counts[@]}"; do
    count=${backend_counts[$backend]}
    percentage=$(awk "BEGIN {printf \"%.1f\", ($count/$total)*100}")
    echo -e "  ${BLUE}${backend}${NC}: ${count}/${total} requests (${percentage}%)"
done

echo ""
echo "Expected: ~33.3% per backend (±3%)"

# Check if distribution is roughly even
max_count=0
min_count=100

for count in "${backend_counts[@]}"; do
    if [ $count -gt $max_count ]; then
        max_count=$count
    fi
    if [ $count -lt $min_count ]; then
        min_count=$count
    fi
done

variance=$((max_count - min_count))

if [ $variance -le 10 ]; then
    echo -e "${GREEN}✓ Distribution looks good (variance: $variance)${NC}"
else
    echo -e "${YELLOW}⚠ Distribution variance is high: $variance${NC}"
    echo "  This might be normal for small sample sizes"
fi
