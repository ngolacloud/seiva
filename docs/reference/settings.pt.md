# Definições e ambiente

O Seiva é configurado inteiramente através do ambiente (12-factor). Esta é a lista
completa das variáveis que o framework e a CLI lêem.

## O serviço (`ServerConfig`, lido no arranque)

| Variável | Significado |
| --- | --- |
| `SEIVA_HTTP_ADDR` | O endereço de escuta, e.g. `0.0.0.0:8080`. |
| `APP_ENV` | `development` ou `production`; conduz a guarda de segurança de produção `AppEnv` (produção recusa valores por omissão só-de-dev). |

Uma configuração inválida ou incompleta faz o arranque falhar (o serviço regista e
sai com código não-zero) em vez de arrancar meio-configurado. Ver
[Configuração](../topics/configuration.md).

## Autenticação (OIDC/JWT)

O servidor constrói um `JwtAuthenticator` quando estas estão definidas; um
conjunto parcial faz o arranque falhar (fail-closed).

| Variável | Significado |
| --- | --- |
| `SEIVA_OIDC_JWKS_FILE` | Caminho para o JWKS usado para verificar tokens. |
| `SEIVA_OIDC_ISSUER` | `iss` esperado. |
| `SEIVA_OIDC_AUDIENCE` | `aud` esperado. |
| `SEIVA_OIDC_TENANT_CLAIM` | Claim que guarda o inquilino (opcional; por omissão `tenant`). |
| `SEIVA_OIDC_PERMISSIONS_CLAIM` | Claim que guarda as permissões (opcional; por omissão `permissions`, com `scope` delimitado por espaços suportado). |

Ver [Autenticação](../topics/authentication.md).

## Camada de dados (DORM)

| Variável | Significado |
| --- | --- |
| `DELONIX_ORM_DSN` | O DSN da base de dados a que a camada de dados se liga — uma string libpq para PostgreSQL/CockroachDB (`host=… dbname=…`) ou um URL para MySQL (`mysql://…`). Lido por `seiva db dev`/`migrate` e pela datasource `data/dorm.schema` gerada. |

Ver [a camada de dados](../topics/data-layer/index.md) e [múltiplas bases de dados](../topics/data-layer/multiple-databases.md).

## CLI / ferramentas

| Variável | Significado |
| --- | --- |
| `SEIVA_EMIT` | Define como `openapi` ou `proto` para fazer a raiz de composição imprimir esse documento e sair em vez de servir (como `seiva openapi`/`proto` documentam um projecto). |
| `SEIVA_FRAMEWORK_DIR` | Sobrepõe o checkout do framework para onde aponta o `[patch.crates-io]` provisório de um projecto gerado (até os crates serem publicados). |
| `SEIVA_DORM_BIN` | O binário da CLI `dorm` a que `seiva db` delega (senão `dorm` no `PATH`). |
| `SEIVA_DORM_DIR` | Um checkout local do DORM para o patch provisório da camada de dados (senão o irmão `../DORM`; ausente → resolve o DORM a partir do crates.io). |

## Relacionado

- [Configuração (guia de tópico)](../topics/configuration.md) · [A CLI `seiva`](cli.md)

---

*Parte da [documentação do Seiva](../index.md).*
