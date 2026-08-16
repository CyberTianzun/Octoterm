import {
  CONTROL_CHANNEL,
  PROTO_VERSION,
  decodeControl,
  decodeFrame,
  encodeControl,
  encodeFrame,
} from "./protocol";

interface AttachState {
  sessionId: number;
  cols: number;
  rows: number;
  lastSeq: number | null;
}

export class OctoClient {
  private ws: WebSocket | null = null;
  private url = "";
  private token = "";
  private backoffMs = 250;
  private closed = false;
  private attachments = new Map<number, AttachState>();

  onControl: (msg: any) => void = () => {};
  onChannelData: (channel: number, payload: Uint8Array) => void = () => {};
  onOpen: () => void = () => {};
  onReconnecting: () => void = () => {};

  connect(url: string, token: string) {
    this.url = url;
    this.token = token;
    this.dial();
  }

  close() {
    this.closed = true;
    this.ws?.close();
  }

  private dial() {
    const ws = new WebSocket(this.url);
    ws.binaryType = "arraybuffer";
    this.ws = ws;
    ws.onopen = () => {
      ws.send(encodeControl({ type: "hello", token: this.token, proto: PROTO_VERSION }));
    };
    ws.onmessage = (ev) => this.handle(ev.data as ArrayBuffer);
    ws.onclose = () => {
      if (this.closed) return;
      this.onReconnecting();
      const delay = this.backoffMs;
      this.backoffMs = Math.min(this.backoffMs * 2, 10_000);
      setTimeout(() => this.dial(), delay);
    };
  }

  private handle(data: ArrayBuffer) {
    const frame = decodeFrame(data);
    if (frame.channel !== CONTROL_CHANNEL) {
      this.onChannelData(frame.channel, frame.payload);
      return;
    }
    const msg = decodeControl(frame.payload) as any;
    switch (msg.type) {
      case "hello-ok": {
        this.backoffMs = 250;
        // 重连后恢复所有 attach
        for (const [channel, a] of this.attachments) {
          this.send({
            type: "attach",
            id: a.sessionId,
            channel,
            last_seq: a.lastSeq,
            cols: a.cols,
            rows: a.rows,
          });
        }
        this.onOpen();
        break;
      }
      case "attached":
        if (msg.mode === "replay") this.trackSeq(msg.channel, msg.seq);
        break;
      case "resync-end":
        this.trackSeq(msg.channel, msg.seq);
        break;
    }
    this.onControl(msg);
  }

  private trackSeq(channel: number, seq: number) {
    const a = this.attachments.get(channel);
    if (a) a.lastSeq = seq;
  }

  send(msg: unknown) {
    this.ws?.send(encodeControl(msg));
  }

  sendInput(channel: number, bytes: Uint8Array) {
    this.ws?.send(encodeFrame(channel, bytes));
  }

  attach(sessionId: number, channel: number, cols: number, rows: number) {
    this.attachments.set(channel, { sessionId, cols, rows, lastSeq: null });
    this.send({ type: "attach", id: sessionId, channel, last_seq: null, cols, rows });
  }

  /** 服务端 Data 帧没带 seq(裸字节),客户端以 resync-end/attached 的 seq 为锚点,
   *  其后每收到 n 字节 lastSeq += n —— 与服务端 end_seq 一致,因为流是连续的。 */
  noteData(channel: number, byteLen: number) {
    const a = this.attachments.get(channel);
    if (a && a.lastSeq !== null) a.lastSeq += byteLen;
  }

  detach(channel: number) {
    this.attachments.delete(channel);
    this.send({ type: "detach", channel });
  }

  resize(channel: number, cols: number, rows: number) {
    const a = this.attachments.get(channel);
    if (a) {
      a.cols = cols;
      a.rows = rows;
    }
    this.send({ type: "resize", channel, cols, rows });
  }
}
