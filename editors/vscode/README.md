# Delonix Seiva — VS Code extension

Editor support for the [Delonix Seiva](https://github.com/ngolacloud) Rust backend
framework. It makes Seiva projects pleasant to read and edit inside VS Code by adding
syntax highlighting for the **DORM schema DSL** and a set of **snippets** for the
most common Seiva and DORM building blocks.

## What it does

### Syntax highlighting — DORM schema (`.schema`)

Registers a `dorm-schema` language for files ending in `.schema` (for example
`schema/<project>.schema` and `data/dorm.schema`). It highlights:

- Block keywords — `datasource`, `generator`, `namespace`, `model`, `enum`, `type`.
- Functions used in defaults/config — `env(...)`, `autoincrement()`, `now()`,
  `uuid()`, `cuid()`, `dbgenerated()`.
- Field attributes — `@id`, `@unique`, `@index`, `@updatedAt`, `@version`,
  `@default(...)`, `@relation(...)`, `@map("...")`, and dotted forms such as
  `@db.VarChar`, `@db.Text`.
- Block attributes — `@@map(...)`, `@@unique(...)`, `@@index(...)`.
- Scalar types — `Uuid`, `String`, `Int`, `BigInt`, `Boolean`, `DateTime`,
  `Decimal`, `Json`, `Inet`, `Cidr`, `MacAddr`.
- String and numeric literals, booleans, the `?` / `[]` type modifiers, and
  `//` line comments.

The language configuration gives you `//` line-comment toggling and
auto-closing / surrounding pairs for `{ }`, `( )`, `[ ]`, and `"`.

### Snippets

Available in `.schema` files:

- `model` — a `model` block with id/createdAt/updatedAt/version and `@@map`.
- `namespace` — a `namespace` block containing a starter model.
- `datasource` — a `datasource db { ... }` block with a provider choice.

Available in Rust files (`.rs`):

- `seiva-facade` — a `#[seiva::facade(version = "v1")]` trait with a
  `#[requires(...)]` method.
- `requires` — a single `#[requires("...")]`-guarded method signature.
- `seiva-controller` — a `#[seiva::controller]` impl block.
- `wire-schema` — a DTO struct deriving `WireSchema`.

## Install (sideload the VSIX)

This extension is not published to the Marketplace. To use it, build a `.vsix`
and install it locally:

```bash
cd editors/vscode
npx @vscode/vsce package        # produces seiva-0.1.0.vsix
code --install-extension seiva-0.1.0.vsix
```

You can also install it from the VS Code UI: open the Extensions view, click the
`...` menu, choose **Install from VSIX...**, and pick the generated file.

## Package

```bash
cd editors/vscode
npx @vscode/vsce package
```

`@vscode/vsce` (the VS Code Extension Manager) is downloaded on demand by `npx`;
network access is required the first time. No login or publisher account is
needed just to **package** a VSIX — only to publish to the Marketplace.

## Scope and limits — honest version

What ships today is **syntax highlighting + snippets only**. That means:

- Highlighting is a TextMate grammar: it is lexical, not semantic. It does not
  validate the schema, resolve relations, or know whether a type is real.
- Snippets are plain text templates; they do not read your project.
- There is **no language server**. The extension does not provide live
  diagnostics, go-to-definition, hover docs, or completion driven by your
  `seiva.toml` / schema.

### Noted next step

A Seiva language server is the natural follow-up: live diagnostics for the DORM
schema (unknown types, duplicate `@id`, dangling `@relation` targets) and for
Rust facades (a `#[requires("...")]` permission that is not declared, a
`#[seiva::facade]` trait without a matching `#[seiva::controller]`, DTOs missing
`#[derive(WireSchema)]`). That work is not part of this extension.
