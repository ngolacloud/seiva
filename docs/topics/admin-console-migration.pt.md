# Migrar para a extensão da consola de administração

A partir da ADR 0008 a consola de administração deixou de fazer parte da base do
Seiva. Se dependias de `GET /admin` ser servido automaticamente, esta página mostra o
que mudou e como o restaurar.

## O que mudou

- **Antes:** todo o backend Seiva servia `/admin` (a consola vivia em `seiva-http`).
- **Depois:** o framework base não carrega código de admin. `GET /admin` é `404`. A
  consola é entregue como a crate opcional **`seiva-console-admin`**, montada
  explicitamente pela tua raiz de composição.
- A `Content-Security-Policy` da consola é agora estrita: o `<style>` e o `<script>`
  inline são fixados por hash SHA-256; o `unsafe-inline` desapareceu.

## Restaurar `/admin` (dois passos + uma recompilação)

Uma dependência declarada por si só não monta rotas — ligas a crate **e** compô-la,
depois recompilas.

1. **Adiciona a dependência** ao pacote da tua raiz de composição (a crate
   `apps/<name>-api`, *não* uma camada `domain`/`application`):

   ```toml
   [dependencies]
   seiva-console-admin = "0.1"
   ```

   Com a ferramenta DX independente isto é `dx add seiva-console-admin`; com o Cargo
   é a linha acima. De qualquer forma a integração é explícita e auditável.

2. **Monta-a no router `modules` ANTES de as camadas do framework a envolverem**, para
   que `/admin` ganhe authn (o `Principal` é populado — o registo depende dele), os
   cabeçalhos de segurança e o limite de taxa como qualquer rota:

   ```rust
   let modules = /* your merged module routers */;
   let modules = seiva_console_admin::mount(modules); // adds GET /admin (+ registry)
   let app = seiva_http::router_with_modules(health, &security, auth, modules);
   ```

   Montá-la sobre o `app` já em camadas (depois de `router_with_modules`) deixaria
   `/admin` fora de todo o middleware — sem authn, pelo que o registo voltaria sempre
   vazio. Um projecto gerado tem uma âncora gerida no sítio certo, e
   `seiva ext add seiva-console-admin` insere a linha aí.

3. **Recompila.** A rota aparece apenas após um rebuild — por desenho.

É tudo. O `seiva_console_admin::mount` adiciona exactamente `GET /admin`; montá-la
duas vezes é um erro em tempo de build (o axum rejeita a rota duplicada), pelo que um
duplo-mount vem à tona no arranque, nunca silenciosamente.

## Se quiseres um backend sem admin

Não faças nada: não adiciones a crate. `/admin` é `404`, e o `cargo tree` não mostra
`seiva-console-admin` no grafo de dependências do backend.

## Compatibilidade

| | Framework base | `seiva-console-admin` |
| --- | --- | --- |
| `0.1.x` | `/admin` → 404 | montar com `seiva_console_admin::mount(router)` |

A consola lê o `/openapi.json` ao vivo do teu serviço, pelo que acompanha os módulos
que o backend montar — sem configuração por consola. (O registo `--admin` por módulo,
o CRUD de linhas de modelo e a autorização da consola do lado do servidor são
incrementos planeados; ver ADR 0008 e o P1-5 do roadmap.)

## Relacionado

- [ADR 0008](../adr/0008-admin-console-as-an-extension.md)
- [Documentos da API](api-documents.md)
