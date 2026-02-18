#!/bin/bash

# Number of nodes
NODES=5

# Clean previous logs
echo "🧹 Cleaning logs..."
rm -rf logs
mkdir -p logs

# Start nodes
echo "🚀 Starting simulation with $NODES nodes..."

for ((i=1; i<=NODES; i++))
do
  echo "▶️ Starting node $i"
  ./node --id=$i --nodes=$NODES > logs/node_$i.log 2>&1 &
done

# Wait a bit for cluster to stabilize
sleep 5

echo "📡 Simulation running..."
echo "Logs available in ./logs"

# Wait for all background processes
wait
