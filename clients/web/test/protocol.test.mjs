import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

// node 直接跑不了 .ts,协议模块保持零依赖纯函数,用 esbuild 即时转译后动态导入
import { execSync } from "node:child_process";
const here = dirname(fileURLToPath(import.meta.url));
execSync("npx esbuild src/protocol.ts --bundle --format=esm --outfile=test/.protocol.build.mjs", {
  cwd: join(here, ".."),
});
const proto = await import("./.protocol.build.mjs");

const fixtures = (name) =>
  JSON.parse(readFileSync(join(here, "../../../crates/protocol/fixtures", name), "utf8"));

test("frame roundtrip", () => {
  const payload = new TextEncoder().encode("hello");
  const buf = proto.encodeFrame(7, payload);
  const view = new DataView(buf);
  assert.equal(view.getUint32(0, true), 7);
  assert.equal(view.getUint8(4), 0);
  const back = proto.decodeFrame(buf);
  assert.equal(back.channel, 7);
  assert.equal(back.flags, 0);
  assert.deepEqual([...back.payload], [...payload]);
});

test("control frames carry fixture messages intact", () => {
  for (const msg of [...fixtures("client-msgs.json"), ...fixtures("server-msgs.json")]) {
    const buf = proto.encodeControl(msg);
    const frame = proto.decodeFrame(buf);
    assert.equal(frame.channel, proto.CONTROL_CHANNEL);
    assert.deepEqual(proto.decodeControl(frame.payload), msg);
  }
});
