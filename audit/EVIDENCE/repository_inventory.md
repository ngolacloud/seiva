# EVIDENCE — Repository Inventory

## Commands run and results

### StillJS — identity & version
```
$ git -C still-js rev-parse HEAD
3ac755d0aa736d3b724da8e3556ba0b254fa9d14
$ git -C still-js log --oneline -1
3ac755d update the changelog and package.json
```
Local `still-js/` is byte-identical to a fresh clone of `github.com/still-js/core` (same HEAD). `package.json`: name `@stilljs/core`, version `1.3.14`, `"type":"module"`, single author, no `test` script, `devDependencies: {http-server}`, scripts invoke `npx live-server`.

### StillJS — module tree (LOC)
```
@still/setup/components.js            1914
@still/component/super/BaseComponent.js 1286
@still/routing/router.js               582
@still/component/super/BehaviorComponent.js 446
@still/helper/template.js              247
@still/component/super/Template.js     203
@still/setup/constants.js               98
@still/setup/error.js                  136
@still/helper/http.js                  108
@still/component/super/AppMixin.js     123
@still/util/componentUtil.js            77
@still/component/manager/registror.js   99
@still/helper/form.js                   76
@still/lone.js                          82
... (total ~5916 LOC JS under @still/)
```

### StillJS — maturity signals
```
$ ls still-js/.github            → NO .github (no CI)
$ grep '"test"' still-js/package.json   → (none)
$ find still-js -name '*.test.js' -o -name '*.spec.js'  → (none)
$ ls still-js/*lock* / package-lock.json  → (none)
```
Entry point: `@still/index.js` loaded by `index.html` via `<script type="module">` into `#stillUiPlaceholder`. Config: `config/app-setup.js`, `config/app-template.js`, `config/route.map.js`.

### Seiva — identity
```
$ git -C seiva rev-parse HEAD
d1effe969a8467888c110e35fe10fa10fe57ebd7
$ git -C seiva log --oneline -1
d1effe9 docs(site): render the documentation as a Django-styled MkDocs site
```

### Seiva — layout
Rust workspace (`resolver=2`). 22 crates under `crates/`: `delonix-seiva`, `seiva-http`, `seiva-console-admin`, `seiva-identity`, `seiva-openapi`, `seiva-auth`, `seiva-macros`, `seiva-server`, `seiva-cli`, `seiva-ports`, `seiva-testing`, `seiva-messaging`, `seiva-ratelimit-redis`, `seiva-config`, `seiva-webhooks`, `seiva-cqrs`, `seiva-grpc`, `seiva-controller-test`, and the DORM-coupled excluded crates (`seiva-data-dorm`, `seiva-outbox-relay`, `seiva-rag-worker`, `seiva-mcp`). `apps/` (admin-e2e, admin-data-e2e, dorm-e2e), `benchmarks/`, `e2e/admin/`, `examples/` (accounts/notes/polls), `docs/` (MkDocs), `editors/vscode/`.

- **Console Admin:** `crates/seiva-console-admin/src/lib.rs` (699 LOC) — the entire admin frontend, server-rendered inline HTML/CSS/JS.
- **Frontend stack:** no client JS framework/bundler anywhere; only `package.json`/`node_modules` are `e2e/admin/` (Playwright) and `editors/vscode/`. No `vite/webpack/rollup/tsconfig`.

## Architectural boundaries

- **StillJS:** tightly coupled global singletons (`$still`, `StillAppSetup`, `window[...]`, `ComponentRegistror`, `WorkerHelper`); `helper/template.js` imports app `config/app-setup.js` (helper↔config coupling). Three component-load paths (ESM `import()`, `<script src>` injection, remote `eval`). No typed contract.
- **Seiva:** clean one-way dependency — `seiva-console-admin` depends on `axum::Router` + `delonix-seiva::admin` + `seiva-identity`; the base (`seiva-http`) does **not** depend on the console (404 for `/admin` unless `mount()`ed).
