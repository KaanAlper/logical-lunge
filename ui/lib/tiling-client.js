// Logical Lunge pencere yöneticisi (lunge-tiling) IPC istemcisi: ws://127.0.0.1:6123.
//
// Bir widget'taki bütün sağlayıcılar tek bağlantıyı paylaşır. Gelen her mesaj bir kez ayrıştırılır; yanıtlar gönderilen
// mesajın metniyle, olaylar abonelik kimliğiyle eşleşir. Bağlantı koparsa bekleyen istekler hatayla biter (hiçbir
// dinleyici geride kalmaz), yanıt gelmeyen istek 10 sn'de zaman aşımına uğrar; yeniden bağlanınca abonelikler
// yenilenir. (Önceki istemci her mesajı her dinleyicide yeniden ayrıştırıyor ve kopmada dinleyicileri sızdırıyordu:
// widget açık kaldıkça iş yükü büyüyordu.)

const URL = 'ws://127.0.0.1:6123';
const REQUEST_TIMEOUT = 10000;

let shared = null;

/** Widget'taki ortak istemci. */
export function getTilingClient() {
  return (shared ??= new TilingClient());
}

export class TilingClient {
  #socket = null;
  #open = false;
  #retryMs = 500;
  #retryTimer = null;
  /** mesaj metni -> sırayla bekleyen istekler */
  #pending = new Map();
  /** yerel abonelik -> { events, callback, serverId } */
  #subscriptions = new Set();
  #connectCallbacks = new Set();
  #disconnectCallbacks = new Set();

  constructor() {
    this.#connect();
  }

  get isConnected() {
    return this.#open;
  }

  /** Bağlanınca (ve her yeniden bağlanışta) çağrılır; zaten bağlıysa hemen. Kaldırma fonksiyonu döner. */
  onConnect(callback) {
    this.#connectCallbacks.add(callback);
    if (this.#open) queueMicrotask(() => this.#connectCallbacks.has(callback) && callback());
    return () => this.#connectCallbacks.delete(callback);
  }

  onDisconnect(callback) {
    this.#disconnectCallbacks.add(callback);
    return () => this.#disconnectCallbacks.delete(callback);
  }

  query(what) {
    return this.#request(`query ${what}`);
  }

  runCommand(command, subjectContainerId) {
    return this.#request(subjectContainerId ? `command --id ${subjectContainerId} ${command}` : `command ${command}`);
  }

  /** events: ['focus_changed', ...]; callback(olay verisi). Kaldırma fonksiyonu döner. */
  subscribe(events, callback) {
    const sub = { events, callback, serverId: null };
    this.#subscriptions.add(sub);
    if (this.#open) this.#sendSubscribe(sub);
    return () => {
      this.#subscriptions.delete(sub);
      if (this.#open && sub.serverId) this.#request(`unsub --id ${sub.serverId}`).catch(() => {});
    };
  }

  #connect() {
    clearTimeout(this.#retryTimer);
    let socket;
    try {
      socket = new WebSocket(URL);
    } catch {
      this.#scheduleRetry();
      return;
    }
    this.#socket = socket;
    socket.onopen = () => {
      this.#open = true;
      this.#retryMs = 500;
      for (const sub of this.#subscriptions) this.#sendSubscribe(sub);
      for (const cb of [...this.#connectCallbacks]) {
        try { cb(); } catch (e) { console.error(e); }
      }
    };
    socket.onmessage = e => this.#onMessage(e.data);
    socket.onclose = () => {
      const wasOpen = this.#open;
      this.#open = false;
      this.#socket = null;
      for (const sub of this.#subscriptions) sub.serverId = null;
      this.#failAll(new Error('Pencere yöneticisi bağlantısı koptu.'));
      if (wasOpen) {
        for (const cb of [...this.#disconnectCallbacks]) {
          try { cb(); } catch (err) { console.error(err); }
        }
      }
      this.#scheduleRetry();
    };
    // onerror'dan sonra her zaman onclose gelir
    socket.onerror = () => {};
  }

  // Pencere yöneticisi yeniden başlarken hızlı dön; açılamıyorsa en fazla 5 sn'de bir dene
  #scheduleRetry() {
    clearTimeout(this.#retryTimer);
    this.#retryTimer = setTimeout(() => this.#connect(), this.#retryMs);
    this.#retryMs = Math.min(5000, this.#retryMs * 2);
  }

  #onMessage(raw) {
    let msg;
    try {
      msg = JSON.parse(raw);
    } catch {
      return;
    }
    if (msg.messageType === 'client_response') {
      const queue = this.#pending.get(msg.clientMessage);
      const req = queue?.shift();
      if (!req) return;
      if (!queue.length) this.#pending.delete(msg.clientMessage);
      clearTimeout(req.timer);
      if (msg.error) req.reject(new Error(msg.error));
      else req.resolve(msg.data);
    } else if (msg.messageType === 'event_subscription') {
      for (const sub of this.#subscriptions) {
        if (sub.serverId === msg.subscriptionId) {
          try { sub.callback(msg.data); } catch (e) { console.error(e); }
        }
      }
    }
  }

  #request(message) {
    if (!this.#open || !this.#socket) return Promise.reject(new Error('Pencere yöneticisine bağlı değil.'));
    return new Promise((resolve, reject) => {
      const req = { resolve, reject, timer: 0 };
      const queue = this.#pending.get(message) ?? [];
      queue.push(req);
      this.#pending.set(message, queue);
      req.timer = setTimeout(() => {
        const q = this.#pending.get(message);
        const i = q ? q.indexOf(req) : -1;
        if (i >= 0) {
          q.splice(i, 1);
          if (!q.length) this.#pending.delete(message);
        }
        reject(new Error(`Pencere yöneticisi yanıt vermedi: ${message}`));
      }, REQUEST_TIMEOUT);
      try {
        this.#socket.send(message);
      } catch (e) {
        clearTimeout(req.timer);
        queue.splice(queue.indexOf(req), 1);
        if (!queue.length) this.#pending.delete(message);
        reject(e);
      }
    });
  }

  async #sendSubscribe(sub) {
    try {
      const data = await this.#request(`sub --events ${sub.events.join(' ')}`);
      // abonelik bu arada kaldırıldıysa sunucudan da kaldır
      if (!this.#subscriptions.has(sub)) {
        this.#request(`unsub --id ${data.subscriptionId}`).catch(() => {});
        return;
      }
      sub.serverId = data.subscriptionId;
    } catch {
      // bağlantı koptu: yeniden bağlanınca tekrar abone olunur
    }
  }

  #failAll(error) {
    for (const queue of this.#pending.values()) {
      for (const req of queue) {
        clearTimeout(req.timer);
        req.reject(error);
      }
    }
    this.#pending.clear();
  }
}
