// A tiny static server for the built storefront (index.html + dist/app.js), on
// its OWN origin so it talks to the shop API cross-origin over CORS. Bind with
// PORT (default 5173).

import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const PORT = Number(process.env.PORT || 5173);
const TYPES = { ".html": "text/html", ".js": "text/javascript" };

createServer(async (req, res) => {
  const path = req.url === "/" || !req.url ? "/index.html" : req.url.split("?")[0];
  const file = join(here, path === "/app.js" ? "dist/app.js" : path.replace(/^\//, ""));
  try {
    const body = await readFile(file);
    const ext = path.slice(path.lastIndexOf("."));
    res.writeHead(200, { "content-type": TYPES[ext] || "application/octet-stream" });
    res.end(body);
  } catch {
    res.writeHead(404);
    res.end("not found");
  }
}).listen(PORT, "127.0.0.1", () => console.log(`storefront on http://127.0.0.1:${PORT}`));
