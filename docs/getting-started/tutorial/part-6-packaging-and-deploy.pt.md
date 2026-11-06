# Parte 6: Empacotamento e deploy

O teu projecto polls nasceu cloud-native. O Seiva compila todo o serviço num único
binário — por isso esta parte constrói a sua imagem e mostra como corre em
produção, em vez de deixar o deploy como um exercício.

## A imagem

`seiva project new` gerou um `Delonixfile` — um build multi-estágio, **distroless**
e não-root:

```dockerfile
FROM rust:1-slim AS builder
WORKDIR /build
COPY . .
RUN cargo build --release --bin polls-site-api

FROM gcr.io/distroless/cc-debian12:nonroot AS runtime
COPY --from=builder /build/target/release/polls-site-api /app/polls-site-api
ENV SEIVA_HTTP_ADDR=0.0.0.0:8080
ENV APP_ENV=production
USER nonroot
ENTRYPOINT ["/app/polls-site-api"]
```

A imagem final carrega **apenas o binário e as suas bibliotecas de runtime** — sem
shell, sem gestor de pacotes — e corre como uid 65532. O deploy é «copiar um
binário», não «enviar um runtime Python + virtualenv + gunicorn».

## Construir e correr

```console
$ delonix stack up        # or `docker compose up` — builds the Delonixfile
```

O `compose.yaml` gerado constrói a partir do `Delonixfile` e mapeia `8080`. Os
serviços de apoio opcionais (o PostgreSQL da Parte 3, um broker) vivem em
`compose/*.yaml` e são activados explicitamente.

## Saúde e prontidão

O serviço expõe `/health/live` e `/health/ready` ao longo de um ciclo de vida
`Starting → Ready → Draining`. O distroless não tem shell para um healthcheck via
CMD, por isso o Kubernetes usa uma sonda de prontidão httpGet em `/health/ready`.
Em `SIGTERM` o serviço esgota os pedidos em curso antes de sair — um rollout é
gracioso.

## Configuração

Tudo é ambiente (12-factor): `SEIVA_HTTP_ADDR`, `APP_ENV`, as variáveis OIDC da
Parte 4, e `DELONIX_ORM_DSN` para a camada de dados da Parte 3. A plataforma injecta
os segredos; nada é cozido na imagem. Ver [Configuração](../../topics/configuration.md).

## Tens um serviço

De `seiva project new` a um serviço polls distroless, autenticado, suportado por
base de dados e auto-documentado — como um único binário compilado. É um
**monólito modular**: um deploy, com o módulo `polls` limpamente delimitado lá
dentro.

## A seguir

[Parte 7: De monólito a microserviço](part-7-monolith-to-microservice.md) — promove
a `polls` ao seu próprio serviço com um comando, o mesmo código de módulo a correr
autónomo. Ou vai mais fundo com os [guias temáticos](../../topics/index.md), ou vê
o [panorama competitivo](../../competitive-landscape.md) para saber onde o Seiva se
posiciona face a outras stacks.

---

*Parte da [documentação do Seiva](../../index.md).*
