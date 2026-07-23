#!/bin/bash

# Store all process IDs.
PIDS=""

# Handle Ctrl+C.
trap "echo ''; echo '🛑 Stopping services...'; kill $PIDS 2>/dev/null; exit 0" INT

# Ensure the binary is executable.
chmod +x polytaoli

# Check the configuration file.
if [ ! -f config.toml ]; then
    echo "❌ Error: config.toml was not found"
    echo "Copy config.example.toml to config.toml and configure it"
    exit 1
fi

# Create the log directory.
mkdir -p logs

# Start the Python order service.
echo "🐍 Starting Python order service (port 8001)..."
cd poly-order-service

# Check the Python configuration.
if [ ! -f config.toml ]; then
    echo "⚠️  Warning: poly-order-service/config.toml does not exist"
    if [ -f config.toml.sample ]; then
        echo "Copy config.toml.sample to config.toml and configure it"
    fi
    echo "The Python order service cannot start"
else
    # Check the Python virtual environment.
    if [ ! -d ".venv" ]; then
        echo "📦 Creating Python virtual environment..."
        python3 -m venv .venv
        source .venv/bin/activate
        pip install -r requirements.txt
    else
        source .venv/bin/activate
    fi
    
    # Start the Python service.
    python main.py &
    PYTHON_PID=$!
    PIDS="$PYTHON_PID"
    echo "✅ Python order service started (PID: $PYTHON_PID)"
    
    # Wait for the Python service to start.
    sleep 3
fi

cd ..

# Start the Rust backend.
echo "🚀 Starting Rust backend (port 8000)..."
./polytaoli &
RUST_PID=$!
PIDS="$PIDS $RUST_PID"
echo "✅ Rust backend started (PID: $RUST_PID)"

echo ""
echo "=================================="
echo "✅ Startup complete!"
echo ""
echo "🐍 Python order service: http://localhost:8001"
echo "📊 Rust backend: http://localhost:8000"
echo ""
echo "Press Ctrl+C to stop all services"
echo "=================================="

# Wait for child processes.
wait
