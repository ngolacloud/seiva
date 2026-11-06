# Como adicionar autenticação

As rotas protegidas já negam por omissão. Para admitir chamadores reais, liga um
autenticador OIDC/JWT.

1. **Obtém o JWKS do teu fornecedor** (as chaves públicas que verificam os seus
   tokens) e guarda-o num ficheiro que o serviço possa ler.

2. **Define o ambiente OIDC** (um conjunto parcial faz o arranque falhar, por
   desenho):

   ```console
   export SEIVA_OIDC_JWKS_FILE=/etc/seiva/jwks.json
   export SEIVA_OIDC_ISSUER=https://id.example.com/
   export SEIVA_OIDC_AUDIENCE=my-api
   # optional: SEIVA_OIDC_TENANT_CLAIM, SEIVA_OIDC_PERMISSIONS_CLAIM
   ```

   O `seiva-server` constrói um `JwtAuthenticator` a partir do JWKS e monta-o
   antes da verificação de permissões de cada rota.

3. **Protege os métodos da tua fachada** com as permissões que os chamadores têm
   de deter:

   ```rust
   #[requires("billing.charge")]
   async fn charge(&self, cmd: Charge) -> Result<ReceiptDto, BillingError>;
   ```

Comportamento: sem bearer → anónimo (as rotas públicas servem, as protegidas
`403`); bearer inválido → `401`; bearer válido → `Principal` verificado,
permissão verificada. Os tokens são apenas RS256/ES256, com `iss`/`aud`/`exp`
validados.

Para verificações por linha (ao nível do objecto), usa `Scoped<T>` — ver
[Autorização](../topics/authorization.md).

## Relacionado

- [Autenticação](../topics/authentication.md) · [Segurança](../topics/security.md) · [Definições](../reference/settings.md)

---

*Parte da [documentação do Seiva](../index.md).*
