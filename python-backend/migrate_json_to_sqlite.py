#!/usr/bin/env python3
"""迁移脚本 - 将现有 JSON 数据导入 SQLite

用法:
    python migrate_json_to_sqlite.py

这个脚本会：
1. 读取 arbitrage_tracking_history.json
2. 创建 SQLite 数据库（如果不存在）
3. 将所有记录和利润历史导入数据库
4. 显示迁移统计信息
"""
import json
import sqlite3
from datetime import datetime
from pathlib import Path
import sys


def datetime_to_ms(dt_str: str) -> int:
    """将 ISO 格式时间字符串转换为毫秒时间戳"""
    try:
        dt = datetime.fromisoformat(dt_str)
        return int(dt.timestamp() * 1000)
    except (ValueError, TypeError):
        return 0


def migrate_json_to_sqlite(
    json_path: str = "arbitrage_tracking_history.json",
    db_path: str = "arbitrage_history.db"
):
    """将 JSON 数据迁移到 SQLite"""
    
    json_file = Path(json_path)
    if not json_file.exists():
        print(f"❌ JSON 文件不存在: {json_path}")
        return False
    
    print(f"📂 读取 JSON 文件: {json_path}")
    print(f"   文件大小: {json_file.stat().st_size / 1024 / 1024:.2f} MB")
    
    # 读取 JSON
    try:
        with open(json_path, 'r', encoding='utf-8') as f:
            data = json.load(f)
    except json.JSONDecodeError as e:
        print(f"❌ JSON 解析失败: {e}")
        return False
    
    completed_records = data.get('completed', [])
    print(f"   记录数量: {len(completed_records)}")
    
    if not completed_records:
        print("⚠️ 没有记录需要迁移")
        return True
    
    # 创建/连接数据库
    print(f"\n📦 创建 SQLite 数据库: {db_path}")
    conn = sqlite3.connect(db_path)
    cursor = conn.cursor()
    
    # 创建表
    cursor.execute("""
        CREATE TABLE IF NOT EXISTS arbitrage_records (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_name TEXT NOT NULL,
            team_name TEXT NOT NULL,
            kalshi_market_id TEXT NOT NULL,
            polymarket_market_id TEXT NOT NULL,
            start_time_ms INTEGER NOT NULL,
            end_time_ms INTEGER,
            max_profit_margin REAL DEFAULT 0,
            max_profit_time_ms INTEGER,
            created_at INTEGER DEFAULT (strftime('%s','now') * 1000)
        )
    """)
    
    cursor.execute("""
        CREATE TABLE IF NOT EXISTS profit_history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            record_id INTEGER NOT NULL,
            time_ms INTEGER NOT NULL,
            profit_margin REAL NOT NULL,
            kalshi_price REAL,
            polymarket_price REAL,
            FOREIGN KEY (record_id) REFERENCES arbitrage_records(id)
        )
    """)
    
    # 创建索引
    cursor.execute("CREATE INDEX IF NOT EXISTS idx_records_event ON arbitrage_records(event_name, team_name)")
    cursor.execute("CREATE INDEX IF NOT EXISTS idx_records_time ON arbitrage_records(start_time_ms)")
    cursor.execute("CREATE INDEX IF NOT EXISTS idx_history_record ON profit_history(record_id)")
    cursor.execute("CREATE INDEX IF NOT EXISTS idx_history_time ON profit_history(time_ms)")
    
    conn.commit()
    
    # 迁移数据
    print("\n🔄 开始迁移数据...")
    
    total_records = 0
    total_history_points = 0
    errors = 0
    
    # 批量插入以提高性能
    batch_size = 100
    
    for i, record in enumerate(completed_records):
        try:
            # 转换时间戳
            start_time_ms = datetime_to_ms(record.get('start_time', ''))
            end_time_ms = datetime_to_ms(record.get('end_time', '')) if record.get('end_time') else None
            max_profit_time_ms = datetime_to_ms(record.get('max_profit_time', '')) if record.get('max_profit_time') else None
            
            # 插入记录
            cursor.execute("""
                INSERT INTO arbitrage_records 
                (event_name, team_name, kalshi_market_id, polymarket_market_id,
                 start_time_ms, end_time_ms, max_profit_margin, max_profit_time_ms)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            """, (
                record.get('event_name', ''),
                record.get('team_name', ''),
                record.get('kalshi_market_id', ''),
                record.get('polymarket_market_id', ''),
                start_time_ms,
                end_time_ms,
                record.get('max_profit_margin', 0),
                max_profit_time_ms
            ))
            
            record_id = cursor.lastrowid
            total_records += 1
            
            # 插入利润历史
            profit_history = record.get('profit_history', [])
            for ph in profit_history:
                time_ms = datetime_to_ms(ph.get('time', ''))
                cursor.execute("""
                    INSERT INTO profit_history 
                    (record_id, time_ms, profit_margin, kalshi_price, polymarket_price)
                    VALUES (?, ?, ?, ?, ?)
                """, (
                    record_id,
                    time_ms,
                    ph.get('profit_margin', 0),
                    ph.get('kalshi_price'),
                    ph.get('polymarket_price')
                ))
                total_history_points += 1
            
            # 定期提交
            if (i + 1) % batch_size == 0:
                conn.commit()
                print(f"   已处理 {i + 1}/{len(completed_records)} 条记录...")
                
        except Exception as e:
            errors += 1
            print(f"   ⚠️ 记录 {i} 迁移失败: {e}")
    
    # 最后提交
    conn.commit()
    
    # 统计结果
    print("\n" + "=" * 50)
    print("📊 迁移完成!")
    print(f"   ✅ 成功导入记录: {total_records}")
    print(f"   ✅ 成功导入历史点: {total_history_points}")
    print(f"   ❌ 失败记录: {errors}")
    
    # 验证
    cursor.execute("SELECT COUNT(*) FROM arbitrage_records")
    db_records = cursor.fetchone()[0]
    
    cursor.execute("SELECT COUNT(*) FROM profit_history")
    db_history = cursor.fetchone()[0]
    
    print(f"\n📦 数据库验证:")
    print(f"   arbitrage_records: {db_records} 条")
    print(f"   profit_history: {db_history} 条")
    
    # 计算数据库大小
    conn.close()
    db_file = Path(db_path)
    if db_file.exists():
        print(f"\n   数据库大小: {db_file.stat().st_size / 1024 / 1024:.2f} MB")
        print(f"   压缩率: {(1 - db_file.stat().st_size / json_file.stat().st_size) * 100:.1f}%")
    
    print("=" * 50)
    return True


def main():
    """主函数"""
    print("=" * 50)
    print("🔄 JSON 到 SQLite 迁移工具")
    print("=" * 50)
    
    # 默认路径
    json_path = "arbitrage_tracking_history.json"
    db_path = "arbitrage_history.db"
    
    # 命令行参数
    if len(sys.argv) > 1:
        json_path = sys.argv[1]
    if len(sys.argv) > 2:
        db_path = sys.argv[2]
    
    success = migrate_json_to_sqlite(json_path, db_path)
    
    if success:
        print("\n✅ 迁移成功!")
        print(f"   现在可以使用新的 SQLite 数据库: {db_path}")
        print("   原 JSON 文件可以备份后删除")
    else:
        print("\n❌ 迁移失败，请检查错误信息")
        sys.exit(1)


if __name__ == "__main__":
    main()
