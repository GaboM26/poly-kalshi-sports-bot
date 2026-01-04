import { Market, ScanResponse } from '../types';

const API_BASE = '/api';

export async function fetchKalshiMarkets(): Promise<Market[]> {
  const response = await fetch(`${API_BASE}/markets/kalshi`);
  if (!response.ok) {
    throw new Error('获取 Kalshi 市场失败');
  }
  return response.json();
}

export async function fetchPolymarketMarkets(): Promise<Market[]> {
  const response = await fetch(`${API_BASE}/markets/polymarket`);
  if (!response.ok) {
    throw new Error('获取 Polymarket 市场失败');
  }
  return response.json();
}

export async function fetchOpportunities(): Promise<ScanResponse> {
  const response = await fetch(`${API_BASE}/opportunities`);
  if (!response.ok) {
    throw new Error('获取套利机会失败');
  }
  return response.json();
}

export async function triggerScan(): Promise<ScanResponse> {
  const response = await fetch(`${API_BASE}/scan`, {
    method: 'POST',
  });
  if (!response.ok) {
    throw new Error('触发扫描失败');
  }
  return response.json();
}

export async function checkHealth(): Promise<{ status: string; version: string }> {
  const response = await fetch(`${API_BASE}/health`);
  if (!response.ok) {
    throw new Error('健康检查失败');
  }
  return response.json();
}
