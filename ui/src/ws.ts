// JSON-RPC 2.0 over WebSocket client (SPEC §9.3).
// Opt-in: existing fetch-based api.ts callers keep working.

type Pending = {
  resolve: (v: unknown) => void;
  reject: (e: Error) => void;
};

type SubHandler = (params: unknown) => void;
export type Unsubscribe = () => void;

const CSRF = (() => {
  const el = document.querySelector('meta[name="tldr-csrf"]') as HTMLMetaElement | null;
  return el?.content ?? "";
})();

export class RpcClient {
  private ws: WebSocket | null = null;
  private nextId = 1;
  private pending = new Map<number, Pending>();
  private subs = new Map<string, SubHandler>();
  private queue: string[] = [];
  private backoff = 500;
  private closed = false;
  private readyWaiters: Array<() => void> = [];

  constructor(private url: string = defaultUrl()) {
    this.connect();
  }

  private connect() {
    if (this.closed) return;
    try {
      this.ws = new WebSocket(this.url);
    } catch {
      this.scheduleReconnect();
      return;
    }
    this.ws.onopen = () => {
      this.backoff = 500;
      const q = this.queue;
      this.queue = [];
      for (const f of q) this.ws?.send(f);
      const waiters = this.readyWaiters;
      this.readyWaiters = [];
      for (const w of waiters) w();
    };
    this.ws.onmessage = (e) => this.onMessage(String(e.data));
    this.ws.onclose = () => this.scheduleReconnect();
    this.ws.onerror = () => { try { this.ws?.close(); } catch { /* ignore */ } };
  }

  private scheduleReconnect() {
    if (this.closed) return;
    const delay = Math.min(this.backoff, 10000);
    this.backoff = Math.min(this.backoff * 2, 10000);
    setTimeout(() => this.connect(), delay);
  }

  private onMessage(raw: string) {
    let msg: { id?: number; result?: unknown; error?: { code: number; message: string }; method?: string; params?: unknown };
    try { msg = JSON.parse(raw); } catch { return; }
    if (typeof msg.id === "number") {
      const p = this.pending.get(msg.id);
      if (!p) return;
      this.pending.delete(msg.id);
      if (msg.error) p.reject(new Error(`rpc ${msg.error.code}: ${msg.error.message}`));
      else p.resolve(msg.result);
      return;
    }
    if (msg.method) {
      const h = this.subs.get(msg.method);
      if (h) h(msg.params);
    }
  }

  private send(frame: string) {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) this.ws.send(frame);
    else this.queue.push(frame);
  }

  call<T = unknown>(method: string, params: unknown = {}): Promise<T> {
    const id = this.nextId++;
    const frame = JSON.stringify({ jsonrpc: "2.0", id, method, params });
    return new Promise<T>((resolve, reject) => {
      this.pending.set(id, { resolve: (v) => resolve(v as T), reject });
      this.send(frame);
    });
  }

  subscribe(method: string, params: unknown, onEvent: SubHandler): Unsubscribe {
    // Map subscribe method to the notification method name the server emits.
    const notifMethod =
      method === "review.subscribe" ? "review.event" :
      method === "index.subscribe" ? "index.event" :
      method === "github.subscribe" ? "github.event" :
      method.replace(/\.subscribe$/, ".event");
    this.subs.set(notifMethod, onEvent);
    this.call(method, params).catch(() => {/* swallow; reconnect will re-subscribe via caller */});
    return () => { this.subs.delete(notifMethod); };
  }

  close() {
    this.closed = true;
    try { this.ws?.close(); } catch { /* ignore */ }
  }
}

function defaultUrl(): string {
  const proto = location.protocol === "https:" ? "wss:" : "ws:";
  return `${proto}//${location.host}/ws?csrf=${encodeURIComponent(CSRF)}`;
}

let singleton: RpcClient | null = null;
export function getRpcClient(): RpcClient {
  if (!singleton) singleton = new RpcClient();
  return singleton;
}
