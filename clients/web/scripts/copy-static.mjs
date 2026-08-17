import { copyFileSync, mkdirSync } from "node:fs";
mkdirSync("dist", { recursive: true });
copyFileSync("src/index.html", "dist/index.html");
copyFileSync("src/style.css", "dist/style.css");
// 全量主题目录:只有打开主题选择器时才 fetch,所以不编进 app.js(见 theme-catalog.ts)
copyFileSync("src/themes/catalog.json", "dist/themes.json");
