Polytaoli - Prediction Market Arbitrage Scanner
================================

Deployment steps:
1. Copy config.example.toml to config.toml.
2. Edit config.toml and enter your Kalshi API credentials.
3. Copy poly-order-service/config.toml.sample to poly-order-service/config.toml.
4. Edit poly-order-service/config.toml and enter the Polymarket private key and wallet address.
5. Run: ./start.sh

Configuration:
- Rust backend port: 8000
- Python order service port: 8001
- Logs: stored in the logs/ directory
- Frontend: visit http://your-server:8000

Service architecture:
- Rust backend: handles arbitrage scanning, WebSockets, and the API
- Python service: handles Polymarket orders (using the official SDK)

Stop the application: Ctrl+C or kill the process.
