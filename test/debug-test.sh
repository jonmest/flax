#!/bin/bash

echo "Starting load balancer in background..."
cd /home/jon/flax
./target/release/flax &
LB_PID=$!

echo "Load balancer PID: $LB_PID"
sleep 2

echo ""
echo "Making test request..."
curl -v http://localhost:3000/small.txt 2>&1 | head -30

echo ""
echo ""
echo "Killing load balancer..."
kill $LB_PID
wait $LB_PID 2>/dev/null
