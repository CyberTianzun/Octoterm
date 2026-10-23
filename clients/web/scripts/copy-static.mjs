import { copyFileSync, mkdirSync, readdirSync } from "node:fs";
mkdirSync("dist/fonts", { recursive: true });
copyFileSync("src/index.html", "dist/index.html");
copyFileSync("src/style.css", "dist/style.css");
// 全量主题目录:只有打开主题选择器时才 fetch,所以不编进 app.js(见 theme-catalog.ts)
copyFileSync("src/themes/catalog.json", "dist/themes.json");
// 内置字体:只拷贝到 dist/fonts/,由 fonts.ts 按需单字体加载进内存
for (const f of readdirSync("src/fonts")) {
  if (f.endsWith(".woff2")) {
    copyFileSync(`src/fonts/${f}`, `dist/fonts/${f}`);
  }
}
