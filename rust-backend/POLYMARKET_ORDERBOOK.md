# Polymarket 订单簿 WebSocket 数据解析

## 重要发现：`best_bid`/`best_ask` 字段是对手方视角

### 问题现象

在 `price_change` 消息中观察到：

```
Heat 消息: best_bid=0.50, best_ask=0.32
Bulls 消息: best_bid=0.68, best_ask=0.50
```

而本地订单簿计算的正确值：
```
Heat: bid=50¢, ask=67¢
Bulls: bid=33¢, ask=50¢
```

### 规律分析

1. **价格互补（和为 1.0）**：
   - Heat 消息 `best_ask` (0.32) + Bulls 消息 `best_bid` (0.68) = 1.0
   - Heat 消息 `best_bid` (0.50) + Bulls 消息 `best_ask` (0.50) = 1.0

2. **消息字段实际含义**：

   | 消息字段 | 实际含义 |
   |---------|---------|
   | `best_bid` | 对手方 token 的 best_ask（反向） |
   | `best_ask` | 对手方 token 的 best_bid（反向） |

### 结论

**消息中的 `best_bid`/`best_ask` 不可信**，必须从本地订单簿计算。

---

## 正确的订单簿维护方式

### 1. 消息格式

**book（初始快照）**：
```json
{
  "event_type": "book",
  "asset_id": "token_id",
  "bids": [{"price": "0.50", "size": "100"}, ...],
  "asks": [{"price": "0.67", "size": "200"}, ...]
}
```

**price_change（增量更新）**：
```json
{
  "event_type": "price_change",
  "price_changes": [{
    "asset_id": "token_id",
    "price": "0.51",
    "size": "50",
    "side": "BUY",
    "best_bid": "0.50",  // ⚠️ 不可信！
    "best_ask": "0.32"   // ⚠️ 不可信！
  }]
}
```

### 2. 订单簿排序规则

- **bids**: 按价格**升序**排列，最后一个是最高买价（best_bid）
- **asks**: 按价格**降序**排列，最后一个是最低卖价（best_ask）

### 3. 代码实现

```rust
// 维护本地订单簿
pub struct PolyOrderBook {
    pub bids: Vec<(f64, f64)>,  // (price, size), 升序
    pub asks: Vec<(f64, f64)>,  // (price, size), 降序
}

impl PolyOrderBook {
    pub fn best_bid(&self) -> Option<(f64, f64)> {
        self.bids.last().copied()  // 最后一个是最高价
    }
    
    pub fn best_ask(&self) -> Option<(f64, f64)> {
        self.asks.last().copied()  // 最后一个是最低价
    }
}
```

### 4. Delta 更新处理

```rust
// 处理 price_change 中的 delta
if let (Some(price), Some(size), Some(side)) = (delta_price, delta_size, delta_side) {
    let book_side = if side == "BUY" { &mut book.bids } else { &mut book.asks };
    
    // 查找价格层级
    if let Some(pos) = book_side.iter().position(|(p, _)| (*p - price).abs() < 0.0001) {
        if size <= 0.0 {
            book_side.remove(pos);  // size=0 表示删除
        } else {
            book_side[pos].1 = size;  // 更新 size
        }
    } else if size > 0.0 {
        book_side.push((price, size));  // 插入新层级
        // 重新排序
    }
}
```

### 5. 获取最佳价格（关键修复）

```rust
// ✅ 正确：始终从本地订单簿获取
let yes_bid = orderbook_cache.read()
    .get(&asset_id)
    .and_then(|b| b.best_bid())
    .map(|(p, _)| p);

let yes_ask = orderbook_cache.read()
    .get(&asset_id)
    .and_then(|b| b.best_ask())
    .map(|(p, _)| p);

// ❌ 错误：使用消息中的 best_bid/best_ask
// let yes_ask = change.get("best_ask").and_then(|v| parse(v));
```

---

## 调试工具

使用 Python 调试脚本验证：

```bash
cd python-backend
python get_poly_tokens.py  # 获取市场 token IDs
python poly_orderbook_monitor.py --token TOKEN_A TOKEN_B --names "Heat" "Bulls" --debug
```

调试输出会显示消息值与本地计算值的对比，并标记差异。
