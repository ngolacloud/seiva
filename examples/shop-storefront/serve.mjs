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

async function send(res, file, type) {
  const body = await readFile(file);
  res.writeHead(200, { "content-type": type });
  res.end(body);
}

createServer(async (req, res) => {
  const path = req.url === "/" || !req.url ? "/index.html" : req.url.split("?")[0];
  try {
    if (path === "/app.js") return await send(res, join(here, "dist/app.js"), TYPES[".js"]);
    const ext = path.slice(path.lastIndexOf("."));
    if (TYPES[ext]) return await send(res, join(here, path.replace(/^\//, "")), TYPES[ext]);
    // No extension → a client-side route (History API); serve the SPA shell so
    // deep links like /product/1 load and the dxjs router resolves them.
    return await send(res, join(here, "index.html"), TYPES[".html"]);
  } catch {
    res.writeHead(404);
    res.end("not found");
  }
}).listen(PORT, "127.0.0.1", () => console.log(`storefront on http://127.0.0.1:${PORT}`));
