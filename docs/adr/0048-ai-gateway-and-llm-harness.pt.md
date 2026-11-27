# ADR 0048 — Gateway de IA e harness de LLM: um só caminho controlado entre um produto e qualquer modelo

- Status: Proposto
- Data: 2026-11-27
- Decisores: NgolaCloud
- Segue: ADR 0006 (identidade), ADR 0013 / 0043 (auditoria), ADR 0015 (rate limit distribuído), ADR 0019 (autorização fail-closed), ADR 0020 (pipeline behaviors), ADR 0025 (WireSchema), ADR 0027 (observabilidade), ADR 0038 (servidor OAuth 2.1), ADR 0045 (guarda de egress)

## Contexto

Os produtos construídos sobre o Seiva pedem agora modelos de linguagem: um chat de
suporte sobre os documentos da empresa, um agente que consulta um cliente e abre um
estorno, uma ferramenta de programação que manda código a um modelo. Cada um deles
envia dados — muitas vezes pessoais, muitas vezes regulados — a um modelo que corre,
em regra, **fora da organização e fora do país**. E cada um deixa um modelo *agir*:
chamar uma ferramenta, ler um registo, escrever um.

Quando cada produto fala sozinho com o modelo, cada produto tem de resolver os
mesmos problemas outra vez, e o mais fraco é a exposição da organização:

- **De quem é o pedido?** A chave do fornecedor é um segredo partilhado. Um produto
  que a tem pode gastá-la em nome de qualquer um; uma chave que se perde é um orçamento
  perdido.
- **O que sai do edifício?** Um prompt com um número de bilhete de identidade, um NIF
  ou um IBAN envia-o a um terceiro noutra jurisdição (a Lei n.º 22/11 de protecção de
  dados pessoais, em Angola, e leis semelhantes noutros países). O mesmo acontece com
  o resultado de uma ferramenta que o agente cola de volta na conversa.
- **O que pode este chamador usar?** Que modelos, que ferramentas, que dados — e se
  uma dada área pode, de todo, mandar dados para fora do país.
- **O que pode o modelo fazer?** Um modelo que pode chamar `refund` pode ser
  convencido a chamá-lo. A injecção de prompt chega pelos documentos e pelos
  resultados das ferramentas, não só pelo utilizador.
- **Quanto?** Os tokens custam dinheiro e capacidade; um ciclo ou um chamador abusivo
  gasta os dois.
- **O que aconteceu?** Quem perguntou o quê, que modelo respondeu, o que foi
  mascarado, que ferramenta correu e com que efeito — com um registo que resiste a
  uma auditoria.
- **E se um controlo partir?** Se o mascaramento, a base do orçamento ou o registo de
  auditoria estiverem em baixo, a resposta segura é «o pedido não sai», não «manda na
  mesma».

A resposta da indústria é um **gateway de IA**: todos os clientes — chat, RAG, agente
de programação, aplicação de secretária — passam por um só serviço dentro da
organização, que aplica identidade, mascaramento, permissões, guardrails, orçamento
e auditoria, falha fechado, e só depois chama o modelo. As ferramentas chegam aos
sistemas internos por **MCP** atrás do mesmo gateway, com a identidade de quem pediu.
Proxies como o LiteLLM tornaram este padrão popular. Em resumo: *autonomia na ponta,
controlo no meio.*

O Seiva já tem a maior parte das peças de controlo: um principal em cada pedido
(ADR 0006), permissões declaradas em cada método de facade e impostas em
fail-closed (ADR 0019), um pipeline tipado de behaviors à volta de um caso de uso
(ADR 0020), auditoria (ADR 0013/0043), rate limiting (ADR 0015), observabilidade
(ADR 0027), um servidor OAuth 2.1 (ADR 0038) e um cliente de saída guardado
(ADR 0045). Falta-lhe o próprio modelo como port, as etapas do gateway que juntam
essas peças no caminho até um modelo, e uma maneira de expor as facades dos módulos
como ferramentas de agentes.

As forças:

- **Levar a ideia, não o produto.** O gateway tem de ser código do Seiva, sob as
  regras de identidade, autorização e auditoria do Seiva — não um proxy externo cujas
  regras o framework não vê nem impõe.
- **Neutro quanto ao fornecedor.** Anthropic, OpenAI, Google, um modelo nas GPUs da
  própria organização ou outro gateway que já esteja na rede — o código de domínio e
  de aplicação não pode saber qual.
- **O mesmo código, duas formas.** Como um módulo (monólito ou microserviço por
  composição), o gateway tem de correr in-process num produto *e* como servidor
  autónomo para o qual outros clientes apontam.
- **Seguro por omissão, extensível por desenho.** Os controlos básicos vêm ligados e
  testados; um programador acrescenta um guardrail, uma regra de mascaramento, um
  fornecedor, uma ferramenta ou um agente sem fazer fork do framework.
- **Mensurável contra uma norma pública.** O OWASP Top 10 for LLM Applications (2025)
  é a lista de aceitação; cada risco é fechado por uma etapa com um teste que o prova.

## Decisão

**O Seiva ganha um port de modelo neutro quanto ao fornecedor, um gateway de IA
construído como um pipeline tipado de etapas fail-closed, ferramentas MCP geradas das
facades com a identidade de quem chama, e um harness de agentes que o programador
estende.** O LiteLLM é a inspiração, nunca uma dependência: nenhuma biblioteca,
proxy ou imagem do LiteLLM é ligada ou distribuída. Um proxy LiteLLM (ou qualquer
gateway compatível com OpenAI) que a organização já tenha é apenas mais um endpoint
na rede — alcançado pelo adaptador compatível com OpenAI através da guarda de
egress, ou posto à frente de um gateway Seiva como um cliente qualquer.

### 1. O port do modelo — `seiva-llm` (sem DORM)

- Tipos neutros: mensagens com papéis e partes de conteúdo (texto, imagem, uso de
  ferramenta, resultado de ferramenta), definições de ferramentas cujo esquema de
  entrada vem do `WireSchema` (ADR 0007/0025), eventos de streaming, uso (tokens de
  entrada, de saída e em cache) e o motivo de paragem.
- Traits: `ChatModel` (completar e stream) e `Embedder`. O worker de RAG liga o
  `Embedder` ao embedder do DORM, por isso um só registo de modelos serve os dois.
- **Rotas, não modelos crus.** Um produto chama uma *rota* (`support-chat`,
  `code-review`), nunca o id de modelo de um fornecedor. Uma rota nomeia uma lista
  ordenada de upstreams (fornecedor + modelo) para fallback, uma `residency`
  (`in_country` ou `abroad`) e a permissão que o chamador tem de ter para a usar.
- Adaptadores atrás de features do cargo: `anthropic`, `openai-compatible` (OpenAI,
  Azure OpenAI, vLLM, Ollama, um proxy LiteLLM existente…), `gemini`. Todos os
  adaptadores ligam pelo cliente guardado do ADR 0045, por isso o endpoint de um
  modelo é um host da allowlist; um modelo na rede da própria organização (um IP
  privado) é uma entrada explícita da allowlist, nunca um desvio implícito.
- Retries e fallback reutilizam o `seiva-resilience`; um fallback nunca atravessa uma
  fronteira de residência (uma rota `in_country` não pode cair para um upstream
  `abroad`).
- As credenciais dos fornecedores vêm de uma `CredentialSource` (ambiente por omissão,
  12-factor; um adaptador Vault atrás de uma feature). Nunca chegam ao chamador e
  nunca aparecem num log, num erro ou numa entrada de auditoria.
- O `seiva-testing` ganha um modelo falso com guião, para que um caso de uso que chama
  um modelo seja testado sem rede.

### 2. O gateway — `seiva-ai-gateway`

Cada controlo é um behavior no sentido do ADR 0020: tipado, composto por aninhamento,
sem registo em runtime. A ordem para uma chamada ao modelo é:

```text
Identify → Authorize → Budget.reserve → Mask → Guard.input
        → model (via route, egress guard)
        → Guard.output → Unmask → Budget.settle → Audit
```

- **Identify.** O chamador é um `Principal` (ADR 0006): uma sessão de utilizador, um
  cliente OAuth (ADR 0038, `client_credentials` para serviços) ou uma chave de API do
  gateway que o Seiva emite e guarda em hash, como qualquer token. A chave de um
  fornecedor nunca é credencial de um chamador.
- **Authorize.** O principal tem de ter a permissão da rota; as ferramentas oferecidas
  ao modelo são a **intersecção** do que a rota permite com o que o principal pode
  chamar. Uma área cujos dados não podem sair do país fica limitada às rotas
  `in_country`. Negar por omissão (ADR 0019).
- **Budget.** Um port `BudgetStore` reserva uma estimativa antes da chamada e acerta o
  uso real depois dela, por principal, por área e por tenant, num período. Ao contrário
  do limitador HTTP (ADR 0015, que falha aberto), **uma falha da base do orçamento
  falha fechado**: uma chamada ao modelo sem medição é despesa que ninguém aprovou.
  Vem uma base em memória; uma base Redis segue o padrão atómico do ADR 0015.
- **Mask.** Detectores substituem os valores sensíveis por marcadores tipados
  (`‹ID_CARD_1›`, `‹TAX_ID_2›`, `‹IBAN_1›`) antes de qualquer coisa sair; o mapa vive em
  memória só para esse pedido ou conversa, nunca gravado em claro. Os detectores
  embutidos cobrem o bilhete de identidade angolano, o NIF e o IBAN (`AO06…`), cartões
  de pagamento (Luhn), e-mail, números de telefone e segredos (chaves, tokens, strings
  de ligação); um produto acrescenta os seus. Numa rota `abroad`, o mascaramento é
  obrigatório e o produto não o pode desligar.
- **Mascarar nas ferramentas.** O resultado de uma ferramenta é mascarado antes de
  entrar na conversa; um argumento de ferramenta com um marcador é desmascarado
  **dentro** da fronteira de confiança, mesmo antes de a ferramenta correr, por isso a
  ferramenta vê o valor real e o modelo nunca o vê.
- **Guard.** Um trait `Guardrail` com ganchos de entrada e de saída. Embutidos: o
  conteúdo de ferramentas e documentos vai embrulhado e marcado como dados não
  confiáveis, nunca como instruções; um limite de tamanho; uma verificação da saída
  que impede um segredo ou dados pessoais por mascarar de chegarem ao chamador; a
  validação do esquema da saída estruturada antes de ser usada. Um produto acrescenta
  os seus (um classificador, uma política de temas).
- **Unmask.** Os marcadores da resposta só são repostos para um chamador com direito
  ao valor original; senão, ficam mascarados.
- **Audit.** Uma entrada por chamada pelo `AuditSink`: principal, rota, modelo
  upstream, tokens, custo, quantos valores foram mascarados (nunca os valores),
  decisões dos guardrails, ferramentas chamadas e o seu efeito. Os prompts e as
  respostas não são guardados por omissão; um produto pode optar por guardar o texto
  **mascarado**. As escritas e os estornos usam a auditoria crítica fail-closed do
  ADR 0043. Os spans seguem as convenções semânticas GenAI do OpenTelemetry
  (ADR 0027).
- **Falha fechada.** Qualquer etapa que dê erro — um detector, a base do orçamento, um
  guardrail, a auditoria crítica — pára o pedido; não há caminho «manda sem mascarar»
  nem flag que o desligue globalmente.

**Duas formas, o mesmo código.** In-process, um produto compõe as etapas à volta do
seu caso de uso (`Gateway::new(route, stages…)`). Como servidor (binário
`seiva-ai-gateway` sobre o `seiva-server`), expõe um `/v1/chat/completions` compatível
com OpenAI, um `/v1/messages` compatível com Anthropic e o `/v1/models`, para que um
chat corporativo, uma interface de RAG ou um agente de programação (Claude Code, um
IDE como o dxtutor-ide) aponte o seu URL base para o gateway e herde todos os
controlos.

### 3. Ferramentas MCP a partir das facades

Um método de facade opta por ser uma ferramenta de agente; nada é exposto por omissão
(agência excessiva, OWASP LLM06):

```rust
#[seiva::facade(version = "v1")]
pub trait Payments {
    /// Refund a captured payment, fully or partly.
    #[requires("payments.refund")]
    #[tool(effect = "write")]
    async fn refund(&self, ctx: &Ctx, cmd: RefundPayment) -> Result<RefundDto, String>;
}
```

- O descritor que já gera o HTTP, o gRPC e o OpenAPI gera a ferramenta MCP: o nome, a
  descrição a partir do comentário de documentação e o esquema de entrada a partir do
  `WireSchema`. O `effect` é obrigatório (`read` ou `write`): o efeito de uma
  ferramenta é um facto de compilação, não um palpite.
- **Em nome da pessoa.** A chamada MCP leva o principal do humano que pediu (token
  exchange, RFC 8693, pelo ADR 0038), nunca um token largo do agente; o `#[requires]`
  e o scope BOLA (ADR 0005) aplicam-se exactamente como no HTTP.
- **As escritas esperam por um humano por omissão.** Uma ferramenta `write` devolve
  uma aprovação pendente que o cliente confirma; um deployment pode pré-aprovar
  ferramentas de escrita nomeadas para principais nomeados, e essa política é
  auditada.
- **Um servidor MCP sem DORM.** Um crate novo, `seiva-mcp-server`, serve as
  ferramentas geradas pelo transporte Streamable HTTP do MCP, atrás da autenticação do
  Seiva, com os metadados de recurso protegido OAuth que os clientes MCP esperam.
  Implementa o subconjunto de servidor de que o Seiva precisa (`initialize`,
  `tools/list`, `tools/call`, `ping`), fixado numa versão do protocolo, com testes de
  conformidade. O `seiva-mcp` actual (as ferramentas de base de dados do DORM, fora do
  workspace) fica como está e passa a ser mais uma fonte de ferramentas.

### 4. O harness de agentes — `seiva-agent`

Um ciclo pequeno que o programador estende em vez de reescrever: enviar → chamadas a
ferramentas → resultados → repetir, limitado por um máximo de passos, tokens, custo e
tempo, com cancelamento. As ferramentas são as geradas das facades, funções Rust
locais ou ferramentas de servidores MCP externos. Cada chamada passa pelas etapas do
gateway. O cliente MCP tem uma allowlist de servidores, fixa a descrição de cada
ferramenta por hash (uma descrição que muda precisa de nova aprovação — o «rug
pull») e trata cada resultado de ferramenta como dados não confiáveis. Os pontos de
extensão são traits: `Tool`, `Guardrail`, `Detector`, `Hook` (antes e depois de uma
chamada a ferramenta) e a base da memória da conversa.

### 5. Recuperação com permissões

O worker de RAG continua a indexar (`seiva-rag-worker`); a recuperação filtra os
pedaços pelo tenant e pelo acesso do chamador **na consulta**, nunca depois dela, e
uma resposta construída com pedaços recuperados leva as suas citações.

### Aceitação: OWASP Top 10 for LLM Applications (2025)

| Risco | Fechado por | Provado por |
|---|---|---|
| LLM01 Injecção de prompt | marcação do conteúdo não confiável; ferramentas limitadas às permissões do principal; escritas aprovadas | um documento injectado que pede um estorno não produz nenhuma escrita |
| LLM02 Divulgação de informação sensível | Mask, mascarar nas ferramentas, guarda da saída | o modelo falso recebe `‹TAX_ID_1›`, nunca o NIF |
| LLM03 Cadeia de fornecimento | `cargo deny`; upstreams e servidores MCP em allowlist; descrições de ferramentas fixadas | uma descrição de ferramenta alterada é recusada até ser aprovada |
| LLM04 Envenenamento de dados e do modelo | a ingestão do RAG regista a fonte e quem escreveu cada pedaço | um pedaço sem proveniência não é indexado |
| LLM05 Tratamento inadequado da saída | validação do esquema da saída estruturada; a saída nunca é interpolada em SQL, HTML ou numa shell | uma saída estruturada inválida é rejeitada antes de ser usada |
| LLM06 Agência excessiva | `#[tool]` opcional, `effect` obrigatório, identidade em nome da pessoa, aprovação das escritas | um token de agente não chama uma ferramenta que o utilizador não pode chamar |
| LLM07 Fuga do system prompt | nada de segredos nem credenciais nos prompts; a guarda da saída bloqueia-os | uma credencial posta num prompt é bloqueada |
| LLM08 Fraquezas de vectores e embeddings | filtro de tenant e de acesso na consulta da recuperação | o tenant A nunca recupera um pedaço do tenant B |
| LLM09 Desinformação | citações nas respostas recuperadas; o produto decide o que uma resposta sem citações pode fazer | uma resposta sem fontes é marcada como tal |
| LLM10 Consumo sem limites | Budget (falha fechado), rate limit, limites de passos, tokens e tempo | a chamada depois de o orçamento acabar é recusada, e todas enquanto a base do orçamento estiver em baixo |

O harness é também revisto contra o OWASP Top 10 for Agentic Applications, a lista
companheira para agentes que planeiam e usam ferramentas.

## Alternativas consideradas

- **Depender do LiteLLM (o proxy Python como sidecar, ou um port dele).** Rejeitado.
  Os seus controlos viveriam fora do principal, das permissões e da auditoria do
  Seiva, por isso o framework não conseguiria provar o que impõe; junta um segundo
  runtime e uma segunda cadeia de fornecimento a cada deployment; e o controlo que
  mais importa aqui — mascaramento com detectores angolanos, residência por área,
  escritas em nome de uma pessoa — teria de ser reconstruído do lado dele de qualquer
  forma. Uma organização que já o tem mantém-no: o adaptador compatível com OpenAI
  alcança-o como upstream.
- **Deixar a cada produto (só um SDK cliente).** Rejeitado: os mesmos controlos
  reescritos por produto, e o produto mais fraco é a exposição da organização — o
  mesmo raciocínio que fez do ADR 0045 um só cliente partilhado.
- **Só servidor, sem modo in-process.** Rejeitado: um monólito pagaria um salto de
  rede e um segundo deployment pelas suas próprias chamadas ao modelo, contra o
  princípio do Seiva «o mesmo código, a composição decide».
- **Um SDK MCP completo para o servidor.** Não agora. Um SDK completo traz o seu
  próprio modelo de transporte e de autenticação, que o Seiva substitui de qualquer
  forma — a razão por que o `seiva-mcp` não usa o HTTP nem o OAuth do DORM. O
  subconjunto de ferramentas é pequeno e tem testes de conformidade; revê-se se o
  Seiva precisar de resources, sampling ou elicitation.
- **Expor automaticamente cada método de facade como ferramenta.** Rejeitado: dá a um
  modelo todas as escritas da API. A opção por método, com um efeito declarado,
  mantém a superfície de ferramentas deliberada e revisível.
- **Mascarar apagando os valores em vez de marcadores.** Rejeitado: o modelo perde a
  estrutura de que precisa («o cliente com `‹ID_CARD_1›`») e uma ferramenta não pode
  ser chamada com o valor real. Marcadores reversíveis dentro da fronteira de
  confiança guardam as duas coisas.

## Consequências

- **Mais fácil.** Um produto chama uma rota e recebe identidade, mascaramento,
  permissões, guardrails, orçamento e auditoria sem esforço; trocar de fornecedor é
  configuração; uma facade existente passa a ferramenta de agente com um atributo e
  mantém a sua permissão e o seu scope de tenant; os chats corporativos e os agentes
  de programação ganham os mesmos controlos ao apontarem para o servidor.
- **Mais difícil.** Cada chamada ao modelo paga as etapas (os detectores são
  expressões regulares e dígitos de controlo sobre o prompt; a base do orçamento é
  uma ida e volta). O mascaramento assenta em padrões e vai falhar o que não conhece:
  os produtos têm de acrescentar detectores para os seus próprios identificadores, e o
  mascaramento reduz, não elimina, o que um modelo vê. O streaming tem de ser
  desmascarado e verificado à saída aos bocados, o que retém alguns tokens nas
  fronteiras dos pedaços.
- **Dívida (datada de 2026-11-27).** Construído por fases, cada uma com os seus testes
  OWASP: G1 `seiva-llm` + adaptadores + modelo falso; G2 as etapas do gateway
  in-process; G3 a forma servidor (compatível com OpenAI e Anthropic); G4 `#[tool]`,
  `seiva-mcp-server`, identidade em nome da pessoa e aprovação das escritas;
  G5 `seiva-agent` e o cliente MCP; G6 recuperação com permissões e citações. Até à
  G4, um agente só usa ferramentas locais.
- **Imposição.** Cada linha da tabela OWASP é um teste no crate que fecha o risco; uma
  fitness function que verifica que cada adaptador do `seiva-llm` liga pelo cliente de
  egress; uma verificação de que uma facade com `#[tool]` declara o `effect`; a ordem
  das etapas do gateway é afirmada por um teste, como a fronteira da transacção no
  ADR 0020.

## Notas de implementação

- **G4 (2026-11-27).** O servidor MCP autentica o token da própria pessoa com o
  `Authenticator` do framework e corre cada ferramenta com o `Ctx` dessa pessoa. A troca de
  tokens (RFC 8693) espera pelo servidor OAuth do ADR 0038; até lá, o cliente apresenta
  directamente o token da pessoa — nunca um token largo de agente. A aprovação humana de uma
  escrita é uma confirmação fora de banda: a aplicação anfitriã chama
  `POST /mcp/approvals/{id}` com o token da pessoa depois de lhe mostrar os argumentos
  exactos, e a nova chamada do modelo leva o id da aprovação; não se usa a elicitation do
  MCP, que precisa de um stream aberto pelo servidor.
- **G6 (2026-11-27).** A recuperação vive no crate `seiva-rag`, sem DORM, atrás de um port
  `VectorIndex` cuja única pesquisa recebe o filtro de acesso de quem chama; vem o índice em
  memória, e o adaptador DORM/pgvector (`seiva-rag-worker`) vai implementar o mesmo port. As
  chamadas de embeddings ainda não passam pelas etapas do gateway: até lá, o guia recomenda
  um modelo de embeddings no país para dados pessoais.

## Relacionadas

- [ADR 0005](0005-object-level-authorization-bola.md) · [ADR 0006](0006-identity-and-authentication.md) · [ADR 0013](0013-admin-mutation-audit.md) · [ADR 0015](0015-distributed-rate-limit.md) · [ADR 0019](0019-fail-closed-authorization.md) · [ADR 0020](0020-command-bus-and-pipeline-behaviors.md) · [ADR 0025](0025-wire-schema-serde-fidelity.md) · [ADR 0027](0027-native-observability.md) · [ADR 0038](0038-oauth-authz-server.md) · [ADR 0043](0043-append-only-audit.md) · [ADR 0045](0045-egress-ssrf-guard.md)

---

*Parte da [documentação do Seiva](../index.md).*
