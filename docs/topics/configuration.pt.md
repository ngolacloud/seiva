# Configuração (12-factor)

Um serviço Seiva lê a sua configuração do ambiente no arranque — um único
`ServerConfig` tipado, validado uma vez, com uma guarda de segurança de produção.
Sem ficheiro de config, sem globais.

## `ServerConfig::from_process_env()`

A raiz de composição chama-o primeiro; perante uma configuração inválida regista em
log e sai com código diferente de zero em vez de arrancar meio-configurado:

```rust
let config = match ServerConfig::from_process_env() {
    Ok(config) => config,
    Err(error) => { tracing::error!(%error, "invalid configuration"); return ExitCode::FAILURE; }
};
```

Variáveis centrais:

- **`SEIVA_HTTP_ADDR`** — o endereço de escuta (p. ex. `0.0.0.0:8080`).
- **`APP_ENV`** — o ambiente (`development` / `production`). Conduz a guarda de
  segurança de produção `AppEnv`: a produção recusa predefinições inseguras que são
  aceitáveis em desenvolvimento.

## Autenticação (OIDC/JWT)

O servidor constrói um `JwtAuthenticator` quando as variáveis OIDC estão definidas;
um conjunto parcial falha o arranque (fail-closed):

- **`SEIVA_OIDC_JWKS_FILE`** — o JWKS contra o qual verificar os tokens;
- **`SEIVA_OIDC_ISSUER`**, **`SEIVA_OIDC_AUDIENCE`** — claims validadas;
- substituições opcionais do nome da claim para tenant e permissões.

Ver [Autenticação](authentication.md).

## Camada de dados

A camada de dados DORM lê o seu DSN de **`DELONIX_ORM_DSN`** (usado pelo
`seiva db dev/migrate` e pela datasource do `data/dorm.schema` gerado). Ver
[a camada de dados](data-layer/index.md) e [múltiplas bases de dados](data-layer/multiple-databases.md).

## A postura twelve-factor

A configuração é ambiente, não código: o mesmo binário corre em dev e prod com
ambiente diferente. A imagem gerada define `APP_ENV=production` e
`SEIVA_HTTP_ADDR=0.0.0.0:8080`, e a plataforma injecta os segredos — nada é embutido.
A lista completa de variáveis está na [referência de definições](../reference/settings.md).

## Relacionado

- [A aplicação e a raiz de composição](transport/application-and-composition.md)
- [Referência de definições](../reference/settings.md) · [Desempenho e cloud-native](performance.md)

---

*Parte da [documentação do Seiva](../index.md).*
