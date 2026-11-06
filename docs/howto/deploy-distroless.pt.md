# Como fazer deploy de uma imagem distroless

Um projecto gerado é cloud-native de origem — a receita é sobretudo "construir a
imagem que ele já tem".

1. **Constrói a imagem.** O `Delonixfile` gerado é um build multi-estágio,
   distroless e non-root:

   ```console
   $ delonix stack up        # or `docker compose up` — builds the Delonixfile
   ```

   A imagem final leva apenas o binário e as suas bibliotecas de runtime (sem
   shell, sem gestor de pacotes) e corre como uid 65532.

2. **Configura via ambiente.** Nada fica embebido: define `SEIVA_HTTP_ADDR`
   (a imagem usa por omissão `0.0.0.0:8080`), `APP_ENV=production`, as variáveis
   OIDC e `DELONIX_ORM_DSN`; a plataforma injecta os segredos. Ver
   [Definições](../reference/settings.md).

3. **Sonda a prontidão com httpGet.** O distroless não tem shell para um
   healthcheck por CMD, por isso usa uma sonda de prontidão httpGet do Kubernetes
   em `/health/ready` (e uma sonda de vivacidade em `/health/live`). Em `SIGTERM`
   o serviço escoa os pedidos em curso, por isso uma actualização rolante é
   graciosa.

## Serviços de apoio por adesão

O `compose.yaml` gerado corre a aplicação; uma base de dados ou broker vive em
`compose/*.yaml` e é incluído explicitamente por adesão:

```console
$ delonix stack up -f compose.yaml -f compose/postgres.yaml
```

## Relacionado

- [Tutorial parte 6](../getting-started/shop/part-9-monolith-to-microservices.md) · [Desempenho e cloud-native](../topics/performance.md)

---

*Parte da [documentação do Seiva](../index.md).*
