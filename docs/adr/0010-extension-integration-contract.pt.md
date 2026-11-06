# ADR 0010 — O contrato de integração de extensões (`dx add` / `seiva ext add`)

- Estado: Aceite
- Data: 2026-11-05
- Decisores: NgolaCloud
- Segue: ADR 0003, ADR 0008, ADR 0009

## Contexto

As ADR 0008/0009 tornaram a consola de administração uma extensão opcional e
registada por módulo. O que faltava era a experiência de **instalação** que o
brief pede:

```
dx add seiva-console-admin
seiva module new inventory --admin
seiva start
```

Uma dependência declarada em `Cargo.toml` **não** chega para inserir routers: a
raiz de composição tem de ligar a crate *e* compô-la (montar o seu router) e
recompilar. O brief é explícito quanto às regras:

- A integração tem de ser **declarativa e auditável**: metadados e operações
  versionados (dependência, feature, import/âncora, mount, config). Nenhum
  `build.rs` ou script de instalação pode correr para descobrir como instalar uma
  extensão.
- Tem de ser **idempotente**: correr duas vezes não duplica nada.
- Para um projecto que a ferramenta **não** reconhece (sem ponto de composição
  gerido), tem de emitir o **patch exacto** e reportar a integração como
  *pendente*, não adivinhar.
- O DX mantém-se uma ferramenta independente, fora deste repo; definimos o
  **contrato** que ele (e o caminho Cargo) consomem, sem copiar o gestor de
  dependências do DX.

## Decisão

**Uma extensão entrega um manifesto declarativo — o contrato de integração — e a
raiz de composição expõe um ponto de composição gerido. Uma ferramenta lê o
manifesto e aplica duas edições (adicionar dependência, inserir mount na âncora);
não executa nenhum script de instalação. `seiva ext add` aplica-o para extensões
de primeira-parte; o DX independente lê a mesma forma de manifesto; o equivalente
Cargo são as mesmas duas edições à mão.**

- **Ponto de composição gerido.** `seiva project new` emite agora duas âncoras
  geridas na raiz de composição: `# seiva:extension-deps` no `Cargo.toml` da app
  em `[dependencies]`, e `// seiva:extensions` em `main.rs` **antes** de `let app
  = router_with_modules(…);`, após os merges de módulos. A instrução de mount
  funde-se no router `modules` pré-camadas (`let modules =
  seiva_console_admin::mount(modules);`), de modo que as rotas da extensão ficam
  envoltas pelas camadas do framework — authn (o `Principal` é populado, do qual o
  registo de administração depende), cabeçalhos de segurança e o rate limit —
  exactamente como uma rota de módulo. Montar sobre o `app` já com camadas
  deixaria as rotas fora de todo o middleware (a regressão que a nota de segurança
  da ADR 0008 e o brief do dono proíbem); a posição da âncora impede-o.
- **Forma do manifesto.** `ExtensionManifest` (em `delonix_seiva::scaffold`):
  `crate_name`, `version_req`, `mount_stmt`, `summary`. A consola entrega os
  mesmos dados como um `seiva-extension.toml` declarativo (`[dependency]`,
  `[compose]`) — o artefacto que o DX lê. Um teste afirma que o TOML entregue e a
  constante embutida concordam, de modo que os dois caminhos de instalação ligam a
  crate de forma idêntica.
- **A aplicação é pura e idempotente.** `add_extension_to_app_manifest` e
  `mount_extension_in_main` inserem nas âncoras e são no-ops quando a
  dependência/mount já está presente (por isso re-correr não muda nada). Uma
  âncora em falta é um erro `MissingAnchor` — o sinal de que o projecto não é uma
  raiz gerida reconhecida.
- **`seiva ext add <name>`.** Resolve o manifesto de primeira-parte, computa
  **ambas** as edições antes de escrever qualquer uma (para que uma âncora em
  falta deixe o projecto intocado), escreve-as, e diz ao utilizador para
  recompilar. Para um projecto não reconhecido, imprime a linha de dependência
  exacta e a instrução de mount a aplicar à mão e sai com um código distinto,
  reportando a integração como pendente. Não corre nenhum script de build ou
  instalação.
- **O registo de primeira-parte é minúsculo e estático.** O `seiva` conhece
  apenas as suas próprias extensões (`seiva-console-admin`); não obtém nem executa
  nada para descobrir como instalar. Extensões de terceiros entregam
  `seiva-extension.toml` para o DX.

## Consequências

**Positivo.**

- `dx add seiva-console-admin` (DX) e `seiva ext add seiva-console-admin`
  (só-Cargo) partilham um contrato e produzem a mesma ligação; os passos manuais
  de Cargo estão documentados e correspondem.
- A instalação é explícita, auditável e recompilada — nunca um hook escondido.
- Re-correr é seguro; uma dependência declarada-mas-não-montada nunca expõe rotas
  silenciosamente.
- Um projecto não gerido mesmo assim recebe um patch exacto e copiável.

**Negativo / custo.**

- A raiz de composição tem de levar as âncoras geridas; um `main.rs` escrito à
  mão que as removeu recorre ao patch manual (por design).
- A lista de extensões embutida do `seiva` é só de primeira-parte; extensões de
  terceiros dependem de o DX ler o seu `seiva-extension.toml` (ou de o utilizador
  aplicar o patch).

## Alternativas consideradas

- **`build.rs`/script de instalação que se liga a si próprio.** Rejeitada
  categoricamente pelo brief: nenhum script corre para descobrir ou realizar a
  instalação.
- **A dependência por si só monta rotas (via um plugin descoberto pelo linker).**
  Rejeitada: torna a exposição implícita e não auditável; a regra de composição
  explícita da ADR 0003 e o portão de autorização da ADR 0009 exigem ambos que o
  mount seja deliberado.
- **Um gestor de pacotes completo no `seiva`.** Rejeitada: o DX já é dono da
  gestão de dependências (um produto separado); o `seiva` define apenas o
  contrato de integração.
