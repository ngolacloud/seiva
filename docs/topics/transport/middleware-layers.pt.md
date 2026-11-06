# Middleware e a pilha de camadas

Cada serviço Seiva envolve as suas rotas numa pilha fixa de middleware de
transporte, aplicada por `router_with_modules`. A ordem é deliberada e segura por
omissão.

## A ordem (a mais externa primeiro)

```
request_id → security_headers → authn → rate limit → CORS → body-size limit → route
```

- **`request_id`** e **`security_headers`** são as camadas **mais externas** de
  propósito: cada resposta carrega um id de correlação e os cabeçalhos seguros (CSP,
  `nosniff`, HSTS, …) **mesmo quando uma camada externa faz curto-circuito** — um
  `429` do rate-limit, um `401` de auth, um preflight CORS. (Montado dentro do
  limitador de taxa, um `429` saiu uma vez nu — medido e corrigido.)
- **`authn`** fica dentro das camadas de cabeçalhos (para que um `401` ainda os
  carregue) mas **fora** das rotas (para que corra antes do `permission_guard` de
  cada rota). Ver [Autenticação](../authentication.md).
- **rate limit**, **CORS** (só quando as origens estão configuradas) e o **limite
  de tamanho do corpo** (1 MiB por omissão) seguem-se.

## A única regra que importa: fundir antes de aplicar as camadas

O `Router::layer` do axum envolve apenas as rotas já presentes. Por isso, fundir as
rotas de um módulo *depois* de aplicar as camadas deixá-las-ia **fora** de todo o
middleware — sem authn, sem cabeçalhos de segurança, sem rate limit.
`router_with_modules` funde os módulos **primeiro** e depois aplica as camadas — a
única ordem que aplica as garantias do framework às rotas dos módulos:

```rust
let modules = Router::new().merge(notes_interface::notes_router(/* … */));
let app = router_with_modules(health.clone(), &security, auth, modules);
```

`seiva module new` liga automaticamente cada módulo neste construtor, por isso
acertas na ordem por omissão. Ver
[a aplicação e a raiz de composição](application-and-composition.md).

## Configurar a postura

`HttpSecurity` controla as defesas ao nível do transporte (OWASP API8): os
cabeçalhos de resposta estão sempre ligados; o CORS é montado só quando defines
origens permitidas; o rate limit é definido por `HttpSecurity`. A opção por omissão
é segura e de mesma-origem. Ver [Segurança](../security.md).

## Relacionado

- [A aplicação e a raiz de composição](application-and-composition.md)
- [Segurança](../security.md) · [Autenticação](../authentication.md)

---

*Parte da [documentação do Seiva](../../index.md).*
