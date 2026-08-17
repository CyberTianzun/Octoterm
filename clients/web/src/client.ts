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
  /** 本次拨号是否已经收到过 hello-ok;每次 dial() 重置。 */
  private authOk = false;
  /** hello-ok 之前收到的 error 消息(比如坏 token),用来判断断线是不是因为
   *  认证失败——认证失败没有重试的意义,重连只会一次次白白触发同样的拒绝。 */
  private preHelloError: string | null = null;

  onControl: (msg: any) => void = () => {};
  onChannelData: (channel: number, payload: Uint8Array) => void = () => {};
  onOpen: () => void = () => {};
  onReconnecting: () => void = () => {};
  /** 认证失败等不可重试的错误:调用后连接不会再自动重连。 */
  onFatal: (message: string) => void = () => {};

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
    this.authOk = false;
    this.preHelloError = null;
    const ws = new WebSocket(this.url);
    ws.binaryType = "arraybuffer";
    this.ws = ws;
    ws.onopen = () => {
      ws.send(encodeControl({ type: "hello", token: this.token, proto: PROTO_VERSION }));
    };
    ws.onmessage = (ev) => this.handle(ev.data as ArrayBuffer);
    ws.onclose = () => {
      if (this.closed) return;
      // 这次拨号连 hello-ok 都没等到,而且服务端在那之前就已经报过错(典型情况:
      // token 不对)——重试没有意义,停止重连并让上层提示用户。
      if (!this.authOk && this.preHelloError !== null) {
        this.closed = true;
        localStorage.removeItem("octoterm-token");
        this.onFatal(this.preHelloError);
        return;
      }
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
      case "error":
        if (!this.authOk) this.preHelloError = msg.message;
        break;
      case "hello-ok": {
        this.authOk = true;
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
        // Attached.seq 只是"重放结束点"的信息量,不是锚点:重放模式下 lastSeq
        // 已经等于本端发出的 last_seq(重连恢复点),后续 noteData 按字节推进即可。
        // 绝不能在这里用 msg.seq 覆盖 lastSeq —— 见 OctoClient 类上方的 seq 记账不变式注释。
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

  /** 服务端 Data 帧没带 seq(裸字节)。lastSeq 只能从控制消息锚定:要么是
   *  Attached{mode:replay} 时本端自己发出的 last_seq(重放字节从该点续接,不
   *  需要、也不应该用 Attached.seq 去覆盖它),要么是 ResyncEnd.seq(resync 的
   *  重绘字节是合成的,不计入账本)。锚定之后,每收到 n 字节 lastSeq += n。 */
  noteData(channel: number, byteLen: number) {
    const a = this.attachments.get(channel);
    if (a && a.lastSeq !== null) a.lastSeq += byteLen;
  }

  detach(channel: number) {
    this.attachments.delete(channel);
    this.send({ type: "detach", channel });
  }

  /** 上报本端想要的尺寸(不是命令:权威尺寸由服务端归并所有 attach 后经
   *  `resized` 下发)。尺寸没变就不发——软键盘弹出/收起会连着触发一串 refit。 */
  resize(channel: number, cols: number, rows: number) {
    const a = this.attachments.get(channel);
    if (!a) return;
    if (a.cols === cols && a.rows === rows) return;
    a.cols = cols;
    a.rows = rows;
    this.send({ type: "resize", channel, cols, rows });
  }
}
