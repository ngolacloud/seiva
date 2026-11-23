# ADR 0032 — `seiva-storage`: um port `ObjectStore` plugável com adapters S3/MinIO e local-fs, por streaming e consciente do tenant

- Status: Proposto
- Data: 2026-11-23
- Decisores: NgolaCloud
- Segue: ADR 0002 (ports do núcleo), ADR 0019 (autorização fail-closed), ADR 0026 (controlo de carga por rota), ADR 0030 (camada de dados plugável)

## Contexto

O Seiva tem uma camada de dados para **registos** e nada para **blobs**. Uma
plataforma precisa de guardar bytes grandes e opacos — gravações, exports,
documentos carregados, avatares, relatórios gerados — e o armazenamento relacional
é o sítio errado para eles. O delonix-meet prova-o: o seu gravador escreve
artefactos IVF/OGG/MP4, a sua analítica emite exports CSV/PDF, os seus perfis
carregam avatares, e hoje cada um desses caminhos abre um cliente S3 à mão,
rederivando credenciais, layout de buckets, prefixo por tenant e presigning no
produto. Isso é um *mecanismo* horizontal — um object store é verdade para qualquer
plataforma — soldado ao código de domínio.

Um ajudante ingénuo que lê um `Vec<u8>` e faz `PUT` é uma armadilha: uma gravação de
2 GB seria bufferizada inteira na memória do processo antes sequer de chegar ao
store. A primitiva tem de ser **por streaming** desde a primeira linha, e tem de ser
**segura quanto ao tenant** desde a primeira linha, porque um blob indexado só por um
nome escolhido pelo produto está a um prefixo em falta de uma leitura entre tenants.

As forças:

- **Um port, não um fornecedor.** O framework tem de expor uma *capacidade*
  `ObjectStore` e deixar o composition root ligar S3, MinIO ou um directório local —
  a mesma linha mecanismo-vs-política do gateway DORM (ADR 0030). Os crates do núcleo
  não podem ligar um AWS SDK por omissão.
- **Streaming, nunca buffer do ficheiro inteiro.** `put`/`get` têm de mover bytes
  como um `AsyncRead`/`AsyncWrite`, para que um objecto grande custe um buffer
  limitado, não o seu tamanho total em RAM. Cargas grandes precisam de **multipart**;
  o processo da aplicação nunca pode ser o estrangulamento de memória de um blob.
- **Consciente do tenant e fail-closed.** Cada chave é resolvida sob um **prefixo por
  tenant** derivado do `Scope` ambiente; uma chamada sem scope **falha fechada** (ADR
  0019), não recai num root partilhado. O adapter local-fs tem de recusar qualquer
  chave que possa sair do seu prefixo de tenant (`..`, absoluta, symlink).
- **A transferência não deve passar pela aplicação.** Para up/download do cliente, o
  store tem de cunhar **presigned URLs** para que os bytes fluam cliente↔storage
  directamente; a aplicação só autoriza e assina, não encaminha o payload.
- **Zero-cost quando não usado e DORM-free.** Um serviço que nada guarda não liga
  nenhum cliente de object store e nada paga; o crate nunca toca a camada de dados.

## Decisão

Adicionar um novo membro do workspace **`seiva-storage`** — sem camada de dados,
atrás de feature, por streaming e consciente do tenant.

- **`ObjectStore`** — o port (um `async_trait`), orientado a bytes e neutro ao
  fornecedor:
  - `put(key, body: impl AsyncRead, meta: ObjectMeta)` e
    `get(key) -> impl AsyncRead` — **por streaming** nos dois sentidos, nunca
    materializando o objecto inteiro.
  - `delete(key)`, `list(prefix) -> stream de ObjectRef`, `head(key) -> ObjectMeta`.
  - `presign_get`/`presign_put(key, ttl) -> Url` — uma URL assinada de TTL curto que
    delega a transferência ao store.
  - `create_multipart`/`upload_part`/`complete_multipart` — para objectos grandes,
    transmitidos parte a parte com memória limitada.
- **`ObjectMeta`** — content-type, tamanho, e um mapa de metadata opcional definido
  pelo produto; o content-type e um limite de tamanho são **validados** antes de um
  `put` concluir.
- **A resolução de `Key` é por tenant.** O store envolve cada chave do chamador sob
  `tenant/{tenant_id}/…` tirado do `Scope` ambiente; um `Scope` `None` é um erro
  fail-closed, não uma escrita num root partilhado. Os blobs dos tenants ficam
  isolados por construção, espelhando o `Scope` da camada de dados (ADR 0030).
- **Adapters, ligados na raiz:**
  - **`S3ObjectStore`** (feature `s3`, `aws-sdk-s3`) — fala S3; funciona contra o AWS
    S3 **e** MinIO/Ceph-RGW via override do endpoint.
  - **`LocalFsObjectStore`** (default, dev) — uma árvore de directórios; cada chave é
    canonicalizada e **confinada** ao seu prefixo de tenant, recusando `..`, caminhos
    absolutos e fuga por symlink.
- **A resiliência é reusada, não reinventada** — as chamadas S3 de saída são
  envolvidas com `seiva-resilience` (backoff do `RetryPolicy` + `timeout()` +
  `CircuitBreaker` opcional); uma carga leva um token de cliente para que um `put`
  repetido seja idempotente em vez de um duplicado.

O `tokio` entra com `default-features = false` (`io-util`, `fs`); o AWS SDK e o seu
runtime são puxados só pela feature `s3`, pelo que o build default fica leve.

## Alternativas consideradas

- **Um único cliente S3 fixo num ajudante `seiva-util`.** Rejeitado: solda o
  framework a um fornecedor, não corre offline em dev ou teste, e não oferece costura
  para MinIO/Ceph ou um directório local. Um port com adapters é a mesma escolha que o
  ADR 0030 fez para a camada de dados.
- **Uma API `bytes`/`Vec<u8>` em memória («só `put(key, data)»).** Rejeitado: faz do
  processo da aplicação o estrangulamento de memória de cada blob e torna gravações
  grandes impossíveis. `AsyncRead`/`AsyncWrite` por streaming + multipart é a única
  forma que limita a memória.
- **Encaminhar cada download pela aplicação.** Rejeitado: queima largura de banda da
  aplicação e um slot de worker por transferência. As presigned URLs deixam o store
  servir bytes directamente enquanto a aplicação mantém a autorização; é mais rápido e
  mais barato.
- **Deixar cada produto prefixar as suas próprias chaves.** Rejeitado: um único
  prefixo esquecido é uma leitura de blob entre tenants. O prefixo por tenant pertence
  ao mecanismo, fail-closed, para que nenhum produto possa optar por sair do
  isolamento.

## Consequências

- **Performance.** `put`/`get` por streaming limitam a memória do processo a um buffer
  independentemente do tamanho do objecto; o multipart carrega objectos grandes parte
  a parte; as presigned URLs movem os bytes reais cliente↔storage, pelo que a
  transferência nunca consome um worker da aplicação. O crate não liga runtime S3 a não
  ser que a feature `s3` esteja ligada — zero-cost quando não usado.
- **Segurança.** Prefixo por tenant derivado do `Scope` ambiente, **fail-closed** num
  scope em falta (ADR 0019); as presigned URLs levam um **TTL curto** e um scope
  apertado de verbo/chave; o adapter local-fs trava a travessia de caminho e a fuga por
  symlink; o content-type e o tamanho são validados antes de uma escrita concluir. As
  credenciais são lidas da configuração, nunca registadas.
- **Resiliência.** As chamadas S3 ganham retry-com-backoff do `seiva-resilience`,
  timeouts por chamada e um circuit breaker opcional, para que um store degradado não
  estagne o serviço; um token de carga do cliente torna um `put` repetido idempotente em
  vez de um objecto duplicado.
- O delonix-meet apaga a sua canalização S3 feita à mão e liga o `S3ObjectStore` (MinIO
  em dev, Ceph-RGW em prod); o gravador e o pipeline de export transmitem artefactos pelo
  port enquanto a **política** — o que se guarda, nomes de buckets, retenção — fica no
  produto.
- **Não** se torna um CDN nem um edge de entrega de media: para assets públicos quentes
  um deployment continua a pôr uma cache à frente do store. Esta é a capacidade de
  objectos in-process, não uma rede de entrega.
