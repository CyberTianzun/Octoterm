import { copyFileSync, mkdirSync } from "node:fs";
mkdirSync("dist", { recursive: true });
copyFileSync("src/index.html", "dist/index.html");
copyFileSync("src/style.css", "dist/style.css");
