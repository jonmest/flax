#!/bin/bash
set -e

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${GREEN}Starting nginx backend servers...${NC}"
docker-compose up -d

echo ""
echo -e "${GREEN}Waiting for backends to be ready...${NC}"
sleep 2

# Check each backend
for port in 8081 8082 8083; do
    if curl -s -f http://localhost:${port}/health > /dev/null; then
        echo -e "${GREEN}✓ Backend on port ${port} is ready${NC}"
    else
        echo -e "${YELLOW}⚠ Backend on port ${port} not responding yet${NC}"
    fi
done

echo ""
echo -e "${GREEN}Backends are running!${NC}"
echo ""
echo "Test with:"
echo "  curl http://localhost:8081/small.txt"
echo "  curl http://localhost:8082/small.txt"
echo "  curl http://localhost:8083/small.txt"
echo ""
echo "View logs:"
echo "  docker-compose logs -f"
echo ""
echo "Stop backends:"
echo "  docker-compose down"
