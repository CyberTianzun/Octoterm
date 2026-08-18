import { test } from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { execSync } from "node:child_process";

// 同 config.test.mjs:node 跑不了 .ts,用 esbuild 即时转译。
const here = dirname(fileURLToPath(import.meta.url));
execSync("npx esbuild src/launchers.ts --bundle --format=esm --outfile=test/.launchers.build.mjs", {
  cwd: join(here, ".."),
});
const { fetchLaunchers, defaultLauncher, providerLabel } = await import("./.launchers.build.mjs");

/** 替掉 globalThis.fetch,返回被记录下来的请求。 */
function stubFetch(handler) {
  const calls = [];
  globalThis.fetch = async (url, init) => {
    calls.push({ url, init });
    return handler();
  };
  return calls;
}

const ok = (body) => () => ({ ok: true, status: 200, json: async () => body });

const sample = {
  launchers: [
    { id: "builtin:default", provider: "builtin", name: "zsh", detail: "/bin/zsh", command: ["/bin/zsh"], cwd: null },
    {
      id: "iterm2:A-1",
      provider: "iterm2",
      name: "Prod SSH",
      detail: "ssh prod01",
      command: ["ssh", "prod01"],
      cwd: "/Users/hiro/work",
    },
  ],
};

test("正常返回时原样解析", async () => {
  stubFetch(ok(sample));
  const list = await fetchLaunchers("tok");
  assert.equal(list.length, 2);
  assert.deepEqual(list[1].command, ["ssh", "prod01"]);
  assert.equal(list[1].cwd, "/Users/hiro/work");
  assert.equal(list[0].cwd, null);
});

test("token 走 Authorization 头,不进 URL", async () => {
  const calls = stubFetch(ok(sample));
  await fetchLaunchers("s3cret");
  assert.equal(calls.length, 1);
  assert.equal(calls[0].url, "/api/launchers");
  assert.equal(calls[0].init.headers.Authorization, "Bearer s3cret");
  assert.ok(!String(calls[0].url).includes("s3cret"), "token 不能出现在 URL 里");
});

// 「新建会话」是最基本的动作,不能因为列表拉不到就用不了 —— 下面几种失败
// 都必须退化成一条兜底项,而不是空菜单或异常。
test("HTTP 失败退化成兜底项", async () => {
  stubFetch(() => ({ ok: false, status: 500, json: async () => ({}) }));
  assert.deepEqual(await fetchLaunchers("tok"), [defaultLauncher()]);
});

test("网络异常退化成兜底项", async () => {
  stubFetch(() => {
    throw new Error("offline");
  });
  assert.deepEqual(await fetchLaunchers("tok"), [defaultLauncher()]);
});

test("空列表退化成兜底项", async () => {
  stubFetch(ok({ launchers: [] }));
  assert.deepEqual(await fetchLaunchers("tok"), [defaultLauncher()]);
});

test("兜底项的 command 为空,表示交给服务端决定", () => {
  assert.deepEqual(defaultLauncher().command, []);
});

test("结构不完整的条目被丢掉,不污染菜单", async () => {
  stubFetch(
    ok({
      launchers: [
        { id: "a", provider: "x", name: "没有 command" },
        { id: "b", provider: "x", name: "command 不是字符串", command: [1, 2] },
        { id: "c", provider: "x", name: "空 command", command: [] },
        { id: "", provider: "x", name: "没有 id", command: ["sh"] },
        { id: "e", provider: "x", name: "", command: ["sh"] },
        { id: "good", provider: "x", name: "好的", command: ["sh"] },
      ],
    }),
  );
  const list = await fetchLaunchers("tok");
  assert.equal(list.length, 1);
  assert.equal(list[0].id, "good");
  // detail 缺失时由 command 拼出来
  assert.equal(list[0].detail, "sh");
});

test("launchers 不是数组时退化成兜底项", async () => {
  stubFetch(ok({ launchers: "nope" }));
  assert.deepEqual(await fetchLaunchers("tok"), [defaultLauncher()]);
  stubFetch(ok({}));
  assert.deepEqual(await fetchLaunchers("tok"), [defaultLauncher()]);
});

test("品牌名不翻译,未知 provider 原样显示", () => {
  assert.equal(providerLabel("iterm2"), "iTerm2");
  assert.equal(providerLabel("windows-terminal"), "Windows Terminal");
  assert.equal(providerLabel("某个插件"), "某个插件");
});
