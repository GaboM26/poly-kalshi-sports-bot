# Polytaoli - Prediction Market Arbitrage Scanner

A high-performance, real-time prediction market arbitrage opportunity monitoring system for **Kalshi** and **Polymarket**.

## Core Features

- **High performance**: Rust backend, React frontend, and real-time WebSocket communication
- **Real-time monitoring**: Synchronized market data from both platforms with intelligent event matching
- **Automated arbitrage**: Calculates profit margins and expected returns, with optional automated order execution
- **Data tracking**: Arbitrage history, position management, and performance monitoring
- **Telegram notifications**: Notifications for automated trades

## System Architecture

```text
+-------------------------------------------------------------+
|                    Frontend (React + TS)                    |
|                  http://localhost:5173                      |
|  - Live arbitrage opportunities - Position management       |
|  - Historical data analysis                                  |
+--------------------+----------------------------------------+
                     | WebSocket + REST API
+--------------------+----------------------------------------+
|                 Rust Backend (Axum + Tokio)                 |
|                  http://localhost:8000                      |
|  +-------------------------------------------------------+  |
|  | Core services                                         |  |
|  | - ArbitrageService: arbitrage calculation and control |  |
|  | - WebSocketManager: live data delivery                |  |
|  | - EventMatcher: intelligent market matching           |  |
|  | - ArbitrageCalculator: profit-margin calculation      |  |
|  | - Storage: SQLite persistence                         |  |
|  +-------------------------------------------------------+  |
|  +------------------+          +-----------------------+  |
|  | Kalshi client    |          | Polymarket client     |  |
|  | - REST API       |          | - REST API            |  |
|  | - WebSocket      |          | - WebSocket           |  |
|  | - RSA signing    |          | - Ethereum signing    |  |
|  +------------------+          +-----------------------+  |
+--------------------+----------------------------------------+
                     | HTTP API
+--------------------+----------------------------------------+
|             Python Order Service (FastAPI)                  |
|                  http://localhost:8001                      |
|  - Uses the official py-clob-client SDK                     |
|  - Handles Polymarket CLOB order signing and submission     |
+-------------------------------------------------------------+
```

### Technology Stack

**Backend (Rust)**

- Axum 0.7 - web framework
- Tokio - asynchronous runtime
- SQLite - data storage
- Reqwest - HTTP client
- Alloy/RSA - cryptographic signing

**Frontend (React)**

- React 18 and TypeScript 5
- Vite 5 - build tool
- Tailwind CSS - styling
- Recharts - data visualization

**Order Service (Python)**

- FastAPI - web framework
- py-clob-client - official Polymarket SDK

## Quick Start

### 1. Prerequisites

- Rust 1.70+
- Node.js 16+
- Python 3.8+

### 2. Configuration

```bash
cd rust-backend
cp config.example.toml config.toml
```

Edit `config.toml`:

```toml
[kalshi]
api_key = "your-kalshi-api-key"
api_secret = """-----BEGIN RSA PRIVATE KEY-----
YOUR_PRIVATE_KEY_HERE
-----END RSA PRIVATE KEY-----"""

[polymarket]
# Obtain from https://reveal.magic.link/polymarket
private_key = "0xYOUR_PRIVATE_KEY"
wallet_address = "0xYOUR_WALLET_ADDRESS"
# Python order service URL
order_service_url = "http://127.0.0.1:8001"

[auth]
username = "admin"
password = "admin123"
secret_key = "your-secret-key-min-32-chars"

[settings]
refresh_interval = 5          # Refresh interval (seconds)
min_profit_margin = 1.0       # Minimum profit margin (%)
default_bet_amount = 10.0     # Default bet amount
tracking_threshold = 2.0      # Tracking threshold (%)

[auto_trade]
enabled = false               # Enable automatic trading
max_amount = 10.0             # Maximum amount per trade
max_trade_count = 2           # Maximum execution count
min_duration_ms = 500         # Minimum opportunity duration

[telegram]
enabled = false
bot_token = "YOUR_BOT_TOKEN"
chat_id = "YOUR_CHAT_ID"
```

### 3. Start All Services

```bash
./start_rust_stack.sh
```

The script starts:

- The Python order service on port 8001
- The Rust backend on port 8000
- The React frontend on port 5173

### 4. Access the Application

- **Frontend**: http://localhost:5173
- **Backend API**: http://localhost:8000
- **Health check**: http://localhost:8000/api/health

Default credentials: `admin` / `admin123`

## Production Deployment

### Linux

```bash
./build_linux.sh
scp polytaoli-linux-x86_64-*.tar.gz user@server:/path/
tar -xzf polytaoli-linux-x86_64-*.tar.gz
cd deploy
cp config.example.toml config.toml
# Edit the configuration, then start the application.
./start.sh
```

### Windows

```bash
./build_windows.sh
```

## Core Functionality

### Arbitrage Calculation

- Calculates profit margins in real time, including fees
- Automatically selects the best strategy: Yes-Yes, Yes-No, No-Yes, or No-No
- Analyzes order book depth

### Market Matching

- Exact matching by event name and question description
- Fuzzy matching by keywords and time range
- Specialized NBA handling that identifies game information intelligently

### Automated Trading

- Filters opportunities by duration threshold
- Configurable trade amount and execution limits
- Real-time Telegram notifications

### Data Management

- SQLite storage for arbitrage history
- Real-time performance metric monitoring
- Position retrieval and management

## API Endpoints

| Endpoint | Method | Description |
| --- | --- | --- |
| `/api/health` | GET | Health check |
| `/api/login` | POST | User authentication |
| `/api/settings` | GET/PUT | Settings management |
| `/api/auto-trade` | GET/PUT | Automated-trading configuration |
| `/api/positions/kalshi` | GET | Kalshi positions |
| `/api/positions/polymarket` | GET | Polymarket positions |
| `/api/arbitrage/history` | GET | Arbitrage history |
| `/api/order/kalshi` | POST | Submit a Kalshi order |
| `/api/order/polymarket` | POST | Submit a Polymarket order |
| `/ws` | WebSocket | Live data delivery |

## Security Recommendations

1. Change the default password and JWT secret.
2. Never commit configuration files that contain real credentials.
3. Use HTTPS with an Nginx reverse proxy in production.
4. Restrict the sources that can access exposed ports.

## Logs

Logs are written to `rust-backend/logs/polytaoli.log.YYYY-MM-DD` and rotate daily.

```bash
tail -f rust-backend/logs/polytaoli.log
```

## Troubleshooting

- **The backend will not start**: Check the configuration-file syntax and API credentials.
- **WebSocket disconnects**: Confirm the backend is running and firewall settings allow access.
- **Data is not updating**: Verify API-key permissions and network connectivity.
- **Automated trades are not executing**: Check the profit-margin threshold and account balance.

---

**Disclaimer**: This software is for educational and research purposes only. You assume all trading risk.
