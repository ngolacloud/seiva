# The CLI and Studio

DORM ships an installable CLI — the `dorm` binary — plus a browser-based data
admin, **DORM Studio**, and a REPL. Inside a Seiva project you mostly reach these
through the `seiva db *` wrappers (see
[Using DORM from Seiva](using-from-seiva.md)), but it is worth knowing the tool
underneath, because `seiva db studio`/`pull` delegate straight to it and the
other commands are available in your project's `data/` directory.

```sh
cargo install dorm-cli        # installs the `dorm` binary
dorm init blog                # scaffold a project
```

## Two lanes

Because DORM is a compile-time ORM, commands come in two lanes:

- **Native** — run inside the installed binary over a DSN, with no project build:
  `doctor`, `db *`, `migrate status`, `migrate resolve`, `completions`, `init`,
  `new model`, `studio`, `format`, `db pull`.
- **Cargo lane** — build and run one of the project's own binaries, because they
  need the compiled models: `validate`, `migrate dev`, `migrate deploy`,
  `migrate diff`, `push`, `shell`.

Global behavior worth knowing: `--dsn` overrides the environment DSN for a run;
`--json` on every read command emits a stable object; exit codes are `0` ok,
`1` error, `2` usage, `3` checks-found-problems; a DSN is never printed with its
password.

## The command surface

| Command | Does | Status |
|---|---|---|
| `dorm init [name]` | Scaffold a project (lib + models + binaries). | done |
| `dorm new model <Name>` | Add a model (struct + registration). | done |
| `dorm generate` | Compile `.schema` → `src/models.rs`. | done |
| `dorm validate` | Check models for mistakes a query/migration would trip over (schema-level, then registry-level). | done |
| `dorm format [--check]` | Pretty-print the schema into its canonical form, losslessly. | done |
| `dorm doctor` | Diagnose config, connectivity, server version, read-only, ledger. | done |
| `dorm db push` / `dorm push` | Sync the schema to the models, **additively** (never drops/retypes). | done |
| `dorm migrate dev [name]` | Author **and** apply the next migration in one step. | done |
| `dorm migrate deploy` (= `dorm migrate`) | Apply the reviewed chain in order under one lock. Idempotent. | done |
| `dorm migrate status` | The applied-versions ledger (`--json`). | done |
| `dorm migrate diff` | Show the additive changes `push` would apply, read-only. | done |
| `dorm migrate resolve <version>` | Mark a version applied without running its SQL — recovery. | done |
| `dorm db tables\|show\|count\|rows` | Browse the live database, read-only (`--json`; `rows --where col=value`). | done |
| `dorm db execute "<SQL>" -p …` | Run explicit SQL with bound parameters (read-only without `--write`). | done |
| `dorm db seed [--file]` | Run a SQL seed file (write path; confirms first). | done |
| `dorm db shell` | Hand you to the native client (`psql`) on the configured database. | done |
| `dorm shell [--write]` | The project REPL (see below). | done |
| `dorm studio [--write]` | The browser data admin (see below). | done |
| `dorm studio generate` | Write a **typed** Studio binary over your models. | done |
| `dorm completions <shell>` | Print a completion script. | done |

> DORM also carries optional, generated surfaces — a RAG worker, an MCP server —
> behind opt-in crates. Under Seiva these responsibilities move up into the
> framework (DORM ADR 0054): a REST API, user accounts, broker clients, and the
> long-running RAG/MCP loops are the framework's layer, not the ORM's. So you
> will not drive them through `dorm` in a Seiva service; the framework owns them.

## The shell (REPL)

`dorm shell` opens a REPL that lists the models the linker gathered and runs
string-keyed queries against a live database — the dynamic query front end in
person. It is read-only unless started with `--write`.

```console
$ dorm shell
dorm> :models
dorm> Post title__icontains=rust limit=5
dorm> Post author__name__icontains=ada        # traverse a relation
```

With `--write` it also runs DML (`insert`/`update`/`delete`) through the compiler
and supports **interactive transactions** (`:begin`/`:commit`/`:rollback`), plus
`:sql` to preview a query's SQL and `:raw` for the parameterized raw valve. Every
filter key is resolved through the model registry before any SQL exists, so an
invented column is an error, not a query. The `dorm-shell` crate can also be
embedded.

## DORM Studio

Studio is a web UI over your live database: browse, filter, search, sort and page
every table, follow foreign keys both ways, and — with `--write` — create, edit
and delete rows, all from the browser. It binds **`127.0.0.1` only**, so it is
never reachable off the machine it runs on.

```sh
export DELONIX_ORM_DSN='host=/var/run/postgresql dbname=blog'
dorm studio --write          # omit --write to browse read-only; --port N to choose the port
#  open  http://127.0.0.1:5555
```

Studio comes in two flavors that share one page and one JSON API:

| | **Native** (`dorm studio`) | **Typed** (`cargo run --bin studio`) |
|---|---|---|
| Learns the schema from | the live database (`information_schema`) | the registry your models populate |
| Needs a project build | no — runs against any PostgreSQL | yes — it links your models |
| Knows relation **names** | no (foreign-key columns only) | yes (reverse relations too) |
| Knows declared type/length/unique | from the catalogue | from the model |

Use the native Studio for a zero-setup look at any database; use the typed Studio
(`dorm studio generate` writes `src/bin/studio.rs`) inside a project to get your
models' own relation names, reverse relations, and declared types.

Studio keeps the ORM's guarantees end to end: every identifier is validated
against the catalogue or registry before it is quoted; every value is a bound
parameter; and an update or delete with no key filter is refused, so the grid
cannot wipe a table by a slip. It opens **read-only** and enables writes only with
`--write` — the same [guard](transactions.md#read-only-mode) the typed API uses.
A bulk delete runs inside one transaction (all selected rows go or none does), and
the CSV export streams the current filtered/sorted view (capped at 100 000 rows).

> `seiva db studio` runs the native Studio for you over your project's DSN; see
> [Using DORM from Seiva](using-from-seiva.md).

## Related

- [Migrations](migrations.md) · [Transactions](transactions.md) ·
  [Backends](backends.md)
- [Using DORM from Seiva](using-from-seiva.md) · the
  [`seiva` CLI reference](../reference/cli.md)

---

*Part of the [Seiva documentation](../index.md).*
