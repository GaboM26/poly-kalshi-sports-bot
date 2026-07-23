#!/bin/bash

# Startup script for the Polytaoli Rust backend, Python order service, and frontend

echo "🚀 Starting Polytaoli (Rust backend version)"
echo "=================================="

# Verify the script is running from the project root.
if [ ! -d "rust-backend" ] || [ ! -d "web" ]; then
    echo "❌ Error: run this script from the project root"
    exit 1
fi

# Store all process IDs.
PIDS=""

# Start the Python Polymarket order service.
echo ""
echo "🐍 Starting Python order service (port 8001)..."
cd poly-order-service

# Check Python dependencies.
if [ ! -f ".venv/bin/python" ]; then
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
echo "⏳ Waiting for the Python service to start..."
sleep 3

# Check the Python service.
if ! curl -s http://localhost:8001/health > /dev/null; then
    echo "⚠️ Warning: the Python order service may not be fully started; continuing..."
fi

cd ..

# Start the Rust backend.
echo ""
echo "📦 Starting Rust backend (port 8000)..."
cd rust-backend

# Check the configuration file.
if [ ! -f "config.toml" ]; then
    echo "⚠️  Warning: config.toml does not exist; copying the example file..."
    if [ -f "config.example.toml" ]; then
        cp config.example.toml config.toml
        echo "✅ Created config.toml; edit it, then run this again"
        kill $PIDS 2>/dev/null
        exit 1
    else
        echo "❌ Error: config.example.toml does not exist either"
        kill $PIDS 2>/dev/null
        exit 1
    fi
fi

# Start the Rust backend in the background.
cargo run --release &
RUST_PID=$!
PIDS="$PIDS $RUST_PID"
echo "✅ Rust backend started (PID: $RUST_PID)"

# Wait for the backend to start.
echo "⏳ Waiting for the backend to start..."
sleep 5

# Check that the backend is healthy.
if ! curl -s http://localhost:8000/api/health > /dev/null; then
    echo "❌ Error: failed to start the Rust backend"
    kill $PIDS 2>/dev/null
    exit 1
fi

echo "✅ Rust backend health check passed"

# Start the frontend.
cd ../web
echo ""
echo "🌐 Starting frontend (port 5173)..."

# Check for node_modules.
if [ ! -d "node_modules" ]; then
    echo "📦 Installing frontend dependencies..."
    npm install
fi

# Start the frontend development server.
npm run dev &
WEB_PID=$!
PIDS="$PIDS $WEB_PID"
echo "✅ Frontend started (PID: $WEB_PID)"

echo ""
echo "=================================="
echo "✅ Startup complete!"
echo ""
echo "🐍 Python order service: http://localhost:8001"
echo "📊 Rust backend: http://localhost:8000"
echo "🌐 Frontend: http://localhost:5173"
echo ""
echo "Press Ctrl+C to stop all services"
echo "=================================="

# Handle Ctrl+C.
trap "echo ''; echo '🛑 Stopping services...'; kill $PIDS 2>/dev/null; exit 0" INT

# Wait for child processes.
wait
