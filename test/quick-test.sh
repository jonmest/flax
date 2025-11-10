#!/bin/bash

GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo "Quick connection test..."
echo ""

# Test backends
echo "1. Testing backend directly..."
if curl -s -f http://localhost:8081/small.txt > /dev/null; then
    echo -e "${GREEN}✓ Backend responding${NC}"
else
    echo -e "${RED}✗ Backend not responding. Run: docker-compose up -d${NC}"
    exit 1
fi

echo ""
echo "2. Testing load balancer..."
response=$(curl -s http://localhost:3000/small.txt 2>&1)
exit_code=$?

if [ $exit_code -eq 0 ]; then
    echo -e "${GREEN}✓ Load balancer is working!${NC}"
    echo "Response: $response"
    echo ""
    echo "Testing with headers..."
    curl -i http://localhost:3000/small.txt | head -15
else
    echo -e "${RED}✗ Load balancer failed (exit code: $exit_code)${NC}"
    echo "Error: $response"
    echo ""
    echo "Make sure it's running: cargo run --release"
    exit 1
fi
