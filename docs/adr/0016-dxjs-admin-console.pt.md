# ADR 0016 — A consola de admin pode ser servida por uma SPA dxjs

- Estado: Aceite
- Data: 2026-11-06
- Decisores: NgolaCloud
- Segue: ADR 0007, ADR 0008, ADR 0009, ADR 0010

## Contexto

A consola de admin é uma extensão opt-in (ADR 0008): o `seiva-console-admin`
serve `GET /admin` como uma página **única, auto-contida e sem build** — HTML/CSS/JS
inline emitido a partir de constantes de texto em Rust, fixado por uma
`Content-Security-Policy` estrita cujos `script-src`/`style-src` carregam o hash
SHA-256 dos bytes inline exactos (sem `unsafe-inline`, sem `unsafe-eval`). Lê o
contrato ao vivo (`/openapi.json`, ADR 0007) cruzado com o
`/admin/registry.json` filtrado por autorização (ADR 0009) e rende tabelas/formulários,
escapando cada valor e ligando handlers com `addEventListener`. A autorização é
imposta no servidor por cada `#[seiva::controller]` dos módulos; as flags `can.*`
da UI são apresentacionais.

Esse desenho é deliberadamente mínimo. À medida que a consola cresce (tabelas de
dados mais ricas, pesquisa/ordenação/paginação do lado do cliente já declaradas no
registo, actualizações ao vivo, edição inline, vistas de diagrama/pipeline), o JS
inline escrito à mão dentro de literais de texto em Rust torna-se o factor
limitante: sem modelo de componentes, sem reactividade, sem testes para o
comportamento no browser, e cada funcionalidade é mais concatenação de texto inline.

Uma auditoria independente (ver o relatório StillJS → Seiva) avaliou adoptar uma
framework Vanilla-JS existente e **rejeitou-a**: o seu motor de renderização era
baseado em `eval()` e não corria sob a CSP estrita do Seiva. Essa auditoria motivou
um motor de raiz, o **dxjs** (`github.com/ngolacloud/dxjs`), construído para a
restrição exacta que esta consola impõe: reactividade de grão fino **sem `eval`,
sem handlers inline, saída escapada por omissão** — provado num browser real sob
`default-src 'none'; script-src 'self'` com zero violações de CSP, e provado
ponta-a-ponta contra o `/admin/registry.json` + `/openapi.json` ao vivo *deste*
backend (a raiz de composição `admin-e2e`) através de um arnês same-origin.

A força em tensão: o Seiva valoriza a consola ser **auto-contida e sem build**
(o ADR 0008 removeu de propósito a sua dependência de CDN), mas uma UX de admin
mais rica quer um motor de componentes real. Uma consola dxjs é um *módulo externo
same-origin* (`<script type="module" src="/admin/dx.js">`), que só precisa de
`script-src 'self'` — continua sem `unsafe-inline`/`unsafe-eval`, por isso a CSP
mantém-se estrita — mas introduz um **bundle JS pré-construído** que o crate tem
de distribuir.

## Decisão

**Adoptar o dxjs como motor de renderização da consola, introduzido como um mount
alternativo opt-in, não como substituição forçada da página inline.**

Em concreto:

- O `seiva-console-admin` continua a servir a página inline, fixada por hash, em
  `GET /admin` como **default e fallback** (inalterada; ainda sem dependências
  extra, ainda com a CSP mais estrita).
- Uma nova superfície opt-in serve a consola dxjs (`seiva_console_admin::mount_dx`
  ou uma feature cargo `dx`) numa rota distinta (p.ex. `GET /admin/dx`), entregando
  um bundle dxjs **commitado e pré-construído** como módulo externo same-origin sob
  `default-src 'none'; script-src 'self'; connect-src 'self'; style-src 'self';
  base-uri 'none'; form-action 'none'; frame-ancestors 'none'` — sem
  `unsafe-inline`, sem `unsafe-eval`, sem origem remota.
- O bundle dxjs é produzido a partir do `examples/seiva-console` no repo dxjs e
  vendorizado neste crate como artefacto commitado com SHA-256 registado; uma
  verificação de CI assegura que os bytes servidos batem com o bundle commitado (a
  propriedade "política e payload nunca podem divergir" do ADR 0008, mantida).
- Tornar o dxjs o `/admin` **default** (e aposentar a página inline) é uma decisão
  posterior e separada, tomada só quando a consola dxjs atingir paridade (tabelas
  de dados, pesquisa/ordenação/paginação, edição/remoção inline, auditoria de
  mutações ADR 0013) e o pipeline de bundle abaixo for aceite.

A autorização, a filtragem do registo (ADR 0009) e o contrato WireSchema (ADR 0007)
ficam inalterados: o dxjs consome os mesmos dois endpoints em modo só-leitura; as
flags `can.*` continuam apresentacionais; o backend permanece a única autoridade.

## Alternativas consideradas

1. **Manter só a consola inline em Rust (status quo).** Sem build, a CSP mais
   estrita possível, nada de novo para manter. Rejeitada como *estado final* porque
   limita a UX da consola: tabelas mais ricas, reactividade e vistas de diagrama são
   impraticáveis como concatenação de texto inline, e o comportamento no browser não
   tem modelo de componentes nem testes. Mantida como **default/fallback**, razão
   pela qual este ADR não a remove.

2. **Substituir `/admin` pela SPA dxjs já (cutover forçado).** Modelo mental mais
   simples, uma só consola. Rejeitada *por agora*: apaga uma implementação bem
   engenhada e sem dependências antes de a substituta ter paridade de
   funcionalidades, e obriga todo o backend Seiva que adopte a consola a também
   carregar um bundle JS. Uma framework não deve dar esse passo irreversível antes
   de a paridade estar demonstrada e o pipeline de bundle aceite. Continua a ser o
   destino pretendido (ver Decisão), condicionado a um ADR de seguimento.

3. **Carregar o dxjs de um CDN.** O crate mais pequeno. Rejeitada de imediato:
   reintroduz a exacta dependência de origem remota que o ADR 0008 removeu e
   exigiria alargar `script-src`/`connect-src` a uma origem de terceiros — contra o
   princípio de consola auto-contida e CSP estrita.

## Consequências

Mais fácil:

- A consola ganha um modelo de componentes real, reactividade, um router e testes
  (o dxjs tem testes unitários e de browser), por isso uma UX de admin mais rica
  (tabelas de dados com chave, pesquisa/ordenação/paginação do cliente a partir das
  allow-lists do registo, actualizações ao vivo via SSE, edição inline) torna-se
  tratável em vez de trabalho de texto inline.
- O motor é CSP-limpo por construção, por isso a propriedade de CSP estrita em que
  o Seiva insiste (ADR 0008) é preservada na nova rota.

Mais difícil / dívida aceite (2026-11-06):

- O `seiva-console-admin` deixa de ser estritamente sem-build para a superfície
  `dx`: passa a distribuir um **bundle JS pré-construído e commitado** produzido
  fora deste repo (dxjs). Dívida: o bundle tem de ser reconstruído e re-vendorizado
  quando a consola dxjs mudar, e a sua proveniência (versão + SHA-256) tem de ser
  registada. Aceitamos isto só para a rota opt-in; o `/admin` default fica sem build.
- Existe uma segunda implementação de consola até um ADR futuro aposentar a inline
  — duplicação temporária, limitada por esse seguimento.
- A rota `dx` usa `script-src 'self'` (módulo externo) em vez de um hash por-byte.
  Continua a ser estrita (sem `unsafe-*`, só same-origin); a garantia de
  integridade passa de um hash inline para uma **verificação de CI de que o bundle
  servido é igual ao artefacto commitado com SHA-256 registado**.

Imposição:

- Um gate de CI neste repo assegura que os bytes do `dx.js` servido batem com o
  bundle commitado e o seu SHA-256 registado (guarda de divergência payload/política,
  por ADR 0008).
- A CI do próprio repo dxjs prova que a consola rende sob CSP estrita (prova de
  browser) e contra o contrato ao vivo deste backend (o arnês `admin-e2e`).
- O `seiva-pattern-fitness` e o `seiva-security-review` correm na alteração do crate
  que adiciona o mount `dx`.

## Veredicto de legibilidade

Legível sem o código à frente: um programador Rust de fora aprende que o `/admin`
continua a página sem-build fixada por hash, que uma SPA dxjs com CSP estrita é
oferecida opt-in como o caminho para uma consola mais rica, exactamente quais
propriedades de segurança são preservadas vs. trocadas (hash por-byte → verificação
de CI do bundle commitado), e que um cutover completo é uma decisão posterior
deliberada — não esta.
