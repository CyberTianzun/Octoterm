export const CONTROL_CHANNEL = 0;
export const PROTO_VERSION = 1;

export interface DecodedFrame {
  channel: number;
  flags: number;
  payload: Uint8Array;
}

export function encodeFrame(channel: number, payload: Uint8Array): ArrayBuffer {
  const buf = new ArrayBuffer(5 + payload.length);
  const view = new DataView(buf);
  view.setUint32(0, channel, true);
  view.setUint8(4, 0);
  new Uint8Array(buf, 5).set(payload);
  return buf;
}

export function decodeFrame(data: ArrayBuffer): DecodedFrame {
  if (data.byteLength < 5) throw new Error("frame shorter than 5-byte header");
  const view = new DataView(data);
  return {
    channel: view.getUint32(0, true),
    flags: view.getUint8(4),
    payload: new Uint8Array(data, 5),
  };
}

export function encodeControl(msg: unknown): ArrayBuffer {
  return encodeFrame(CONTROL_CHANNEL, new TextEncoder().encode(JSON.stringify(msg)));
}

export function decodeControl(payload: Uint8Array): unknown {
  return JSON.parse(new TextDecoder().decode(payload));
}
