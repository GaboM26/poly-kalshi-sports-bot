"""异步存储服务 - SQLite 套利记录存储

使用异步队列解耦业务层和存储层，避免存储操作阻塞业务逻辑。
时间精度：毫秒级
"""
import asyncio
import sqlite3
import logging
import threading
from datetime import datetime
from typing import Optional, List, Dict, Any
from dataclasses import dataclass
from enum import Enum
from pathlib import Path

logger = logging.getLogger(__name__)


class StorageEventType(Enum):
    """存储事件类型"""
    START_TRACKING = "start_tracking"
    UPDATE_TRACKING = "update_tracking"
    END_TRACKING = "end_tracking"
    ADD_PROFIT_HISTORY = "add_profit_history"


@dataclass
class StorageEvent:
    """存储事件"""
    event_type: StorageEventType
    data: Dict[str, Any]
    timestamp_ms: int


def datetime_to_ms(dt: datetime) -> int:
    """将 datetime 转换为毫秒时间戳"""
    return int(dt.timestamp() * 1000)


def ms_to_datetime(ms: int) -> datetime:
    """将毫秒时间戳转换为 datetime"""
    return datetime.fromtimestamp(ms / 1000.0)


def now_ms() -> int:
    """获取当前毫秒时间戳"""
    return int(datetime.now().timestamp() * 1000)


class ArbitrageStorage:
    """套利记录 SQLite 存储服务
    
    特点：
    1. 异步队列：业务层 put_nowait() 不阻塞
    2. 批量写入：减少 IO 操作
    3. 毫秒精度：时间戳精确到毫秒
    4. 优雅关闭：停止时 flush 剩余数据
    """
    
    def __init__(self, db_path: str = "arbitrage_history.db"):
        self.db_path = db_path
        self._queue: asyncio.Queue[StorageEvent] = asyncio.Queue()
        self._running = False
        self._worker_task: Optional[asyncio.Task] = None
        self._conn: Optional[sqlite3.Connection] = None
        self._lock = threading.Lock()
        
        # 活跃追踪记录的数据库 ID 映射: tracking_key -> record_id
        self._active_record_ids: Dict[str, int] = {}
        
        # 批量写入配置
        self._batch_size = 50
        self._batch_timeout_ms = 100
        
        # 初始化数据库
        self._init_db()
    
    def _init_db(self):
        """初始化数据库表结构"""
        self._conn = sqlite3.connect(self.db_path, check_same_thread=False)
        self._conn.row_factory = sqlite3.Row
        
        cursor = self._conn.cursor()
        
        # 套利记录主表
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
        
        # 利润历史表
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
        
        self._conn.commit()
        logger.info(f"📦 SQLite 数据库初始化完成: {self.db_path}")
    
    async def start(self):
        """启动存储 Worker"""
        if self._running:
            return
        
        self._running = True
        self._worker_task = asyncio.create_task(self._worker_loop())
        logger.info("🚀 存储 Worker 已启动")
    
    async def stop(self):
        """停止存储 Worker，flush 剩余数据"""
        if not self._running:
            return
        
        self._running = False
        
        # 等待队列清空
        logger.info(f"⏳ 正在 flush 剩余 {self._queue.qsize()} 条存储事件...")
        
        # 发送一个结束信号
        await self._queue.put(None)
        
        if self._worker_task:
            try:
                await asyncio.wait_for(self._worker_task, timeout=5.0)
            except asyncio.TimeoutError:
                logger.warning("⚠️ Worker 关闭超时")
                self._worker_task.cancel()
        
        # 关闭数据库连接
        if self._conn:
            self._conn.close()
            self._conn = None
        
        logger.info("🛑 存储 Worker 已停止")
    
    async def _worker_loop(self):
        """Worker 循环：批量处理存储事件"""
        batch: List[StorageEvent] = []
        last_flush_time = now_ms()
        
        while self._running or not self._queue.empty():
            try:
                # 尝试获取事件，超时后检查是否需要 flush
                try:
                    event = await asyncio.wait_for(
                        self._queue.get(),
                        timeout=self._batch_timeout_ms / 1000.0
                    )
                    
                    # 收到结束信号
                    if event is None:
                        break
                    
                    batch.append(event)
                    
                except asyncio.TimeoutError:
                    pass
                
                # 检查是否需要 flush
                current_time = now_ms()
                should_flush = (
                    len(batch) >= self._batch_size or
                    (batch and current_time - last_flush_time >= self._batch_timeout_ms)
                )
                
                if should_flush and batch:
                    self._flush_batch(batch)
                    batch = []
                    last_flush_time = current_time
                    
            except Exception as e:
                logger.error(f"❌ Worker 处理异常: {e}")
        
        # 最后 flush 剩余数据
        if batch:
            self._flush_batch(batch)
    
    def _flush_batch(self, batch: List[StorageEvent]):
        """批量写入数据库"""
        if not batch:
            return
        
        with self._lock:
            cursor = self._conn.cursor()
            
            try:
                for event in batch:
                    self._process_event(cursor, event)
                
                self._conn.commit()
                logger.debug(f"💾 批量写入 {len(batch)} 条存储事件")
                
            except Exception as e:
                logger.error(f"❌ 批量写入失败: {e}")
                self._conn.rollback()
    
    def _process_event(self, cursor: sqlite3.Cursor, event: StorageEvent):
        """处理单个存储事件"""
        data = event.data
        
        if event.event_type == StorageEventType.START_TRACKING:
            # 插入新记录
            cursor.execute("""
                INSERT INTO arbitrage_records 
                (event_name, team_name, kalshi_market_id, polymarket_market_id, 
                 start_time_ms, max_profit_margin, max_profit_time_ms)
                VALUES (?, ?, ?, ?, ?, ?, ?)
            """, (
                data['event_name'],
                data['team_name'],
                data['kalshi_market_id'],
                data['polymarket_market_id'],
                data['start_time_ms'],
                data['profit_margin'],
                data['start_time_ms']
            ))
            
            record_id = cursor.lastrowid
            tracking_key = data['tracking_key']
            self._active_record_ids[tracking_key] = record_id
            
            # 插入第一条利润历史
            cursor.execute("""
                INSERT INTO profit_history (record_id, time_ms, profit_margin, kalshi_price, polymarket_price)
                VALUES (?, ?, ?, ?, ?)
            """, (
                record_id,
                data['start_time_ms'],
                data['profit_margin'],
                data.get('kalshi_price'),
                data.get('polymarket_price')
            ))
            
        elif event.event_type == StorageEventType.UPDATE_TRACKING:
            tracking_key = data['tracking_key']
            record_id = self._active_record_ids.get(tracking_key)
            
            if record_id:
                # 更新最大利润
                if data.get('is_new_max'):
                    cursor.execute("""
                        UPDATE arbitrage_records 
                        SET max_profit_margin = ?, max_profit_time_ms = ?
                        WHERE id = ?
                    """, (data['profit_margin'], data['time_ms'], record_id))
            
        elif event.event_type == StorageEventType.ADD_PROFIT_HISTORY:
            tracking_key = data['tracking_key']
            record_id = self._active_record_ids.get(tracking_key)
            
            if record_id:
                cursor.execute("""
                    INSERT INTO profit_history (record_id, time_ms, profit_margin, kalshi_price, polymarket_price)
                    VALUES (?, ?, ?, ?, ?)
                """, (
                    record_id,
                    data['time_ms'],
                    data['profit_margin'],
                    data.get('kalshi_price'),
                    data.get('polymarket_price')
                ))
            
        elif event.event_type == StorageEventType.END_TRACKING:
            tracking_key = data['tracking_key']
            record_id = self._active_record_ids.get(tracking_key)
            
            if record_id:
                cursor.execute("""
                    UPDATE arbitrage_records SET end_time_ms = ? WHERE id = ?
                """, (data['end_time_ms'], record_id))
                
                # 从活跃记录中移除
                del self._active_record_ids[tracking_key]
    
    # ========== 业务层调用接口（非阻塞）==========
    
    def track_start(
        self,
        tracking_key: str,
        event_name: str,
        team_name: str,
        kalshi_market_id: str,
        polymarket_market_id: str,
        profit_margin: float,
        kalshi_price: float = None,
        polymarket_price: float = None
    ):
        """开始追踪（非阻塞）"""
        time_ms = now_ms()
        event = StorageEvent(
            event_type=StorageEventType.START_TRACKING,
            data={
                'tracking_key': tracking_key,
                'event_name': event_name,
                'team_name': team_name,
                'kalshi_market_id': kalshi_market_id,
                'polymarket_market_id': polymarket_market_id,
                'start_time_ms': time_ms,
                'profit_margin': profit_margin,
                'kalshi_price': kalshi_price,
                'polymarket_price': polymarket_price
            },
            timestamp_ms=time_ms
        )
        
        try:
            self._queue.put_nowait(event)
        except asyncio.QueueFull:
            logger.warning("⚠️ 存储队列已满，丢弃 START 事件")
    
    def track_update(
        self,
        tracking_key: str,
        profit_margin: float,
        is_new_max: bool = False,
        kalshi_price: float = None,
        polymarket_price: float = None
    ):
        """更新追踪（非阻塞）"""
        time_ms = now_ms()
        
        # 添加利润历史
        history_event = StorageEvent(
            event_type=StorageEventType.ADD_PROFIT_HISTORY,
            data={
                'tracking_key': tracking_key,
                'time_ms': time_ms,
                'profit_margin': profit_margin,
                'kalshi_price': kalshi_price,
                'polymarket_price': polymarket_price
            },
            timestamp_ms=time_ms
        )
        
        try:
            self._queue.put_nowait(history_event)
        except asyncio.QueueFull:
            logger.warning("⚠️ 存储队列已满，丢弃 HISTORY 事件")
        
        # 如果是新高，更新记录
        if is_new_max:
            update_event = StorageEvent(
                event_type=StorageEventType.UPDATE_TRACKING,
                data={
                    'tracking_key': tracking_key,
                    'time_ms': time_ms,
                    'profit_margin': profit_margin,
                    'is_new_max': True
                },
                timestamp_ms=time_ms
            )
            
            try:
                self._queue.put_nowait(update_event)
            except asyncio.QueueFull:
                logger.warning("⚠️ 存储队列已满，丢弃 UPDATE 事件")
    
    def track_end(self, tracking_key: str):
        """结束追踪（非阻塞）"""
        time_ms = now_ms()
        event = StorageEvent(
            event_type=StorageEventType.END_TRACKING,
            data={
                'tracking_key': tracking_key,
                'end_time_ms': time_ms
            },
            timestamp_ms=time_ms
        )
        
        try:
            self._queue.put_nowait(event)
        except asyncio.QueueFull:
            logger.warning("⚠️ 存储队列已满，丢弃 END 事件")
    
    # ========== 查询接口 ==========
    
    def get_completed_records(self, limit: int = 100, offset: int = 0) -> List[Dict]:
        """获取已完成的套利记录"""
        with self._lock:
            cursor = self._conn.cursor()
            cursor.execute("""
                SELECT * FROM arbitrage_records 
                WHERE end_time_ms IS NOT NULL
                ORDER BY start_time_ms DESC
                LIMIT ? OFFSET ?
            """, (limit, offset))
            
            rows = cursor.fetchall()
            return [dict(row) for row in rows]
    
    def get_active_records(self) -> List[Dict]:
        """获取活跃的套利记录"""
        with self._lock:
            cursor = self._conn.cursor()
            cursor.execute("""
                SELECT * FROM arbitrage_records 
                WHERE end_time_ms IS NULL
                ORDER BY start_time_ms DESC
            """)
            
            rows = cursor.fetchall()
            return [dict(row) for row in rows]
    
    def get_profit_history(self, record_id: int, limit: int = 1000) -> List[Dict]:
        """获取指定记录的利润历史"""
        with self._lock:
            cursor = self._conn.cursor()
            cursor.execute("""
                SELECT * FROM profit_history 
                WHERE record_id = ?
                ORDER BY time_ms ASC
                LIMIT ?
            """, (record_id, limit))
            
            rows = cursor.fetchall()
            return [dict(row) for row in rows]
    
    def get_record_with_history(self, record_id: int) -> Optional[Dict]:
        """获取记录及其完整利润历史"""
        with self._lock:
            cursor = self._conn.cursor()
            
            # 获取记录
            cursor.execute("SELECT * FROM arbitrage_records WHERE id = ?", (record_id,))
            row = cursor.fetchone()
            
            if not row:
                return None
            
            record = dict(row)
            
            # 获取利润历史
            cursor.execute("""
                SELECT time_ms, profit_margin, kalshi_price, polymarket_price
                FROM profit_history 
                WHERE record_id = ?
                ORDER BY time_ms ASC
            """, (record_id,))
            
            record['profit_history'] = [dict(r) for r in cursor.fetchall()]
            
            return record
    
    def get_stats(self) -> Dict:
        """获取统计信息"""
        with self._lock:
            cursor = self._conn.cursor()
            
            cursor.execute("SELECT COUNT(*) FROM arbitrage_records WHERE end_time_ms IS NOT NULL")
            completed_count = cursor.fetchone()[0]
            
            cursor.execute("SELECT COUNT(*) FROM arbitrage_records WHERE end_time_ms IS NULL")
            active_count = cursor.fetchone()[0]
            
            cursor.execute("SELECT COUNT(*) FROM profit_history")
            history_count = cursor.fetchone()[0]
            
            return {
                'completed_count': completed_count,
                'active_count': active_count,
                'total_history_points': history_count,
                'queue_size': self._queue.qsize()
            }
    
    def get_all_completed_with_history(self, limit: int = 100) -> List[Dict]:
        """获取所有已完成记录及其利润历史（用于 API）"""
        with self._lock:
            cursor = self._conn.cursor()
            
            # 获取已完成记录
            cursor.execute("""
                SELECT * FROM arbitrage_records 
                WHERE end_time_ms IS NOT NULL
                ORDER BY start_time_ms DESC
                LIMIT ?
            """, (limit,))
            
            records = []
            for row in cursor.fetchall():
                record = dict(row)
                
                # 转换时间戳为 ISO 格式
                record['start_time'] = ms_to_datetime(record['start_time_ms']).isoformat()
                record['end_time'] = ms_to_datetime(record['end_time_ms']).isoformat() if record['end_time_ms'] else None
                record['max_profit_time'] = ms_to_datetime(record['max_profit_time_ms']).isoformat() if record['max_profit_time_ms'] else None
                record['duration_seconds'] = (record['end_time_ms'] - record['start_time_ms']) / 1000.0 if record['end_time_ms'] else None
                
                # 获取利润历史
                cursor.execute("""
                    SELECT time_ms, profit_margin, kalshi_price, polymarket_price
                    FROM profit_history 
                    WHERE record_id = ?
                    ORDER BY time_ms ASC
                """, (record['id'],))
                
                record['profit_history'] = [
                    {
                        'time': ms_to_datetime(r['time_ms']).isoformat(),
                        'profit_margin': r['profit_margin'],
                        'kalshi_price': r['kalshi_price'],
                        'polymarket_price': r['polymarket_price']
                    }
                    for r in cursor.fetchall()
                ]
                
                records.append(record)
            
            return records
    
    def search_records(
        self,
        min_profit: Optional[float] = None,
        max_profit: Optional[float] = None,
        min_duration: Optional[float] = None,
        max_duration: Optional[float] = None,
        event_name: Optional[str] = None,
        team_name: Optional[str] = None,
        start_date: Optional[str] = None,
        end_date: Optional[str] = None,
        sort_by: str = "start_time",
        sort_order: str = "desc",
        limit: int = 100,
        offset: int = 0,
        include_history: bool = False
    ) -> Dict:
        """高级搜索历史记录
        
        Args:
            min_profit: 最小利润率
            max_profit: 最大利润率
            min_duration: 最小持续时间（秒）
            max_duration: 最大持续时间（秒）
            event_name: 事件名称（模糊匹配）
            team_name: 队伍名称（模糊匹配）
            start_date: 开始日期（ISO格式）
            end_date: 结束日期（ISO格式）
            sort_by: 排序字段（start_time, max_profit_margin, duration）
            sort_order: 排序方向（asc, desc）
            limit: 返回数量
            offset: 偏移量
            include_history: 是否包含利润历史
            
        Returns:
            包含 records, total, page_info 的字典
        """
        with self._lock:
            cursor = self._conn.cursor()
            
            # 构建查询条件
            conditions = ["end_time_ms IS NOT NULL"]
            params = []
            
            if min_profit is not None:
                conditions.append("max_profit_margin >= ?")
                params.append(min_profit)
            
            if max_profit is not None:
                conditions.append("max_profit_margin <= ?")
                params.append(max_profit)
            
            if min_duration is not None:
                conditions.append("(end_time_ms - start_time_ms) >= ?")
                params.append(min_duration * 1000)  # 转换为毫秒
            
            if max_duration is not None:
                conditions.append("(end_time_ms - start_time_ms) <= ?")
                params.append(max_duration * 1000)  # 转换为毫秒
            
            if event_name:
                conditions.append("event_name LIKE ?")
                params.append(f"%{event_name}%")
            
            if team_name:
                conditions.append("team_name LIKE ?")
                params.append(f"%{team_name}%")
            
            if start_date:
                try:
                    start_ms = datetime_to_ms(datetime.fromisoformat(start_date.replace('Z', '+00:00')))
                    conditions.append("start_time_ms >= ?")
                    params.append(start_ms)
                except ValueError:
                    pass
            
            if end_date:
                try:
                    end_ms = datetime_to_ms(datetime.fromisoformat(end_date.replace('Z', '+00:00')))
                    conditions.append("start_time_ms <= ?")
                    params.append(end_ms)
                except ValueError:
                    pass
            
            where_clause = " AND ".join(conditions)
            
            # 排序字段映射
            sort_map = {
                "start_time": "start_time_ms",
                "max_profit_margin": "max_profit_margin",
                "duration": "(end_time_ms - start_time_ms)",
                "event_name": "event_name",
                "team_name": "team_name"
            }
            order_field = sort_map.get(sort_by, "start_time_ms")
            order_dir = "DESC" if sort_order.lower() == "desc" else "ASC"
            
            # 获取总数
            cursor.execute(f"SELECT COUNT(*) FROM arbitrage_records WHERE {where_clause}", params)
            total = cursor.fetchone()[0]
            
            # 获取记录
            query = f"""
                SELECT *, (end_time_ms - start_time_ms) / 1000.0 as duration_seconds
                FROM arbitrage_records 
                WHERE {where_clause}
                ORDER BY {order_field} {order_dir}
                LIMIT ? OFFSET ?
            """
            cursor.execute(query, params + [limit, offset])
            
            records = []
            for row in cursor.fetchall():
                record = dict(row)
                
                # 转换时间戳为 ISO 格式
                record['start_time'] = ms_to_datetime(record['start_time_ms']).isoformat()
                record['end_time'] = ms_to_datetime(record['end_time_ms']).isoformat() if record['end_time_ms'] else None
                record['max_profit_time'] = ms_to_datetime(record['max_profit_time_ms']).isoformat() if record['max_profit_time_ms'] else None
                
                # 获取利润历史（可选）
                if include_history:
                    cursor.execute("""
                        SELECT time_ms, profit_margin, kalshi_price, polymarket_price
                        FROM profit_history 
                        WHERE record_id = ?
                        ORDER BY time_ms ASC
                    """, (record['id'],))
                    
                    record['profit_history'] = [
                        {
                            'time': ms_to_datetime(r['time_ms']).isoformat(),
                            'profit_margin': r['profit_margin'],
                            'kalshi_price': r['kalshi_price'],
                            'polymarket_price': r['polymarket_price']
                        }
                        for r in cursor.fetchall()
                    ]
                else:
                    record['profit_history'] = []
                
                records.append(record)
            
            return {
                "records": records,
                "total": total,
                "limit": limit,
                "offset": offset,
                "has_more": offset + limit < total
            }
    
    def get_statistics(self) -> Dict:
        """获取历史记录统计信息"""
        with self._lock:
            cursor = self._conn.cursor()
            
            # 基础统计
            cursor.execute("""
                SELECT 
                    COUNT(*) as total_records,
                    AVG(max_profit_margin) as avg_profit,
                    MAX(max_profit_margin) as max_profit,
                    MIN(max_profit_margin) as min_profit,
                    AVG((end_time_ms - start_time_ms) / 1000.0) as avg_duration,
                    MAX((end_time_ms - start_time_ms) / 1000.0) as max_duration,
                    MIN((end_time_ms - start_time_ms) / 1000.0) as min_duration,
                    SUM((end_time_ms - start_time_ms) / 1000.0) as total_duration
                FROM arbitrage_records
                WHERE end_time_ms IS NOT NULL
            """)
            row = cursor.fetchone()
            
            # 按事件统计
            cursor.execute("""
                SELECT event_name, COUNT(*) as count, AVG(max_profit_margin) as avg_profit
                FROM arbitrage_records
                WHERE end_time_ms IS NOT NULL
                GROUP BY event_name
                ORDER BY count DESC
                LIMIT 10
            """)
            top_events = [dict(r) for r in cursor.fetchall()]
            
            # 按队伍统计
            cursor.execute("""
                SELECT team_name, COUNT(*) as count, AVG(max_profit_margin) as avg_profit
                FROM arbitrage_records
                WHERE end_time_ms IS NOT NULL
                GROUP BY team_name
                ORDER BY count DESC
                LIMIT 10
            """)
            top_teams = [dict(r) for r in cursor.fetchall()]
            
            # 利润分布
            cursor.execute("""
                SELECT 
                    CASE 
                        WHEN max_profit_margin < 3 THEN '0-3%'
                        WHEN max_profit_margin < 5 THEN '3-5%'
                        WHEN max_profit_margin < 10 THEN '5-10%'
                        WHEN max_profit_margin < 20 THEN '10-20%'
                        ELSE '20%+'
                    END as range,
                    COUNT(*) as count
                FROM arbitrage_records
                WHERE end_time_ms IS NOT NULL
                GROUP BY range
                ORDER BY 
                    CASE range
                        WHEN '0-3%' THEN 1
                        WHEN '3-5%' THEN 2
                        WHEN '5-10%' THEN 3
                        WHEN '10-20%' THEN 4
                        ELSE 5
                    END
            """)
            profit_distribution = [dict(r) for r in cursor.fetchall()]
            
            # 持续时间分布（毫秒级精度）
            cursor.execute("""
                SELECT 
                    CASE 
                        WHEN (end_time_ms - start_time_ms) < 1000 THEN '<1s'
                        WHEN (end_time_ms - start_time_ms) < 5000 THEN '1-5s'
                        WHEN (end_time_ms - start_time_ms) < 10000 THEN '5-10s'
                        WHEN (end_time_ms - start_time_ms) < 30000 THEN '10-30s'
                        WHEN (end_time_ms - start_time_ms) < 60000 THEN '30s-1m'
                        WHEN (end_time_ms - start_time_ms) < 300000 THEN '1-5m'
                        WHEN (end_time_ms - start_time_ms) < 600000 THEN '5-10m'
                        ELSE '10m+'
                    END as range,
                    COUNT(*) as count,
                    AVG(max_profit_margin) as avg_profit
                FROM arbitrage_records
                WHERE end_time_ms IS NOT NULL
                GROUP BY range
                ORDER BY 
                    CASE range
                        WHEN '<1s' THEN 1
                        WHEN '1-5s' THEN 2
                        WHEN '5-10s' THEN 3
                        WHEN '10-30s' THEN 4
                        WHEN '30s-1m' THEN 5
                        WHEN '1-5m' THEN 6
                        WHEN '5-10m' THEN 7
                        ELSE 8
                    END
            """)
            duration_distribution = [dict(r) for r in cursor.fetchall()]
            
            # 持续时间百分位数
            cursor.execute("""
                SELECT (end_time_ms - start_time_ms) as duration_ms
                FROM arbitrage_records
                WHERE end_time_ms IS NOT NULL
                ORDER BY duration_ms
            """)
            durations = [r[0] for r in cursor.fetchall()]
            
            percentiles = {}
            if durations:
                n = len(durations)
                percentiles = {
                    "p50": durations[int(n * 0.5)] if n > 0 else 0,
                    "p75": durations[int(n * 0.75)] if n > 0 else 0,
                    "p90": durations[int(n * 0.9)] if n > 0 else 0,
                    "p95": durations[int(n * 0.95)] if n > 0 else 0,
                    "p99": durations[int(n * 0.99)] if n > 0 else 0,
                }
            
            return {
                "total_records": row[0] or 0,
                "avg_profit": round(row[1] or 0, 2),
                "max_profit": round(row[2] or 0, 2),
                "min_profit": round(row[3] or 0, 2),
                "avg_duration": round(row[4] or 0, 1),
                "avg_duration_ms": round((row[4] or 0) * 1000, 0),
                "max_duration": round(row[5] or 0, 1),
                "max_duration_ms": round((row[5] or 0) * 1000, 0),
                "min_duration": round(row[6] or 0, 1),
                "min_duration_ms": round((row[6] or 0) * 1000, 0),
                "total_duration": round(row[7] or 0, 1),
                "duration_percentiles": percentiles,
                "top_events": top_events,
                "top_teams": top_teams,
                "profit_distribution": profit_distribution,
                "duration_distribution": duration_distribution
            }


# 全局单例
_storage_instance: Optional[ArbitrageStorage] = None


def get_storage(db_path: str = "arbitrage_history.db") -> ArbitrageStorage:
    """获取存储服务单例"""
    global _storage_instance
    if _storage_instance is None:
        _storage_instance = ArbitrageStorage(db_path)
    return _storage_instance
