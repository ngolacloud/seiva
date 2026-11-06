# Paginação

Para endpoints de listagem, prefere paginação por **keyset** (cursor) à paginação
por offset.

## Porquê keyset

A paginação por offset (`LIMIT n OFFSET m`) é instável sob escritas concorrentes (as
linhas deslocam-se entre páginas) e degrada-se em páginas profundas (a base de dados
continua a percorrer as linhas saltadas). A paginação por keyset carrega um
**cursor** sobre uma ordenação estável e lê a página seguinte com um
`WHERE (key) > (cursor)` — estável e O(página), não O(offset).

## No Seiva

A convenção de rota `list` (um `GET /{version}/{resource}` cujo único DTO de
consulta carrega o cursor e o limite — ver
[controladores e encaminhamento](transport/controllers-and-routing.md)) é onde a
paginação vive no fio. O repositório transforma-a numa leitura delimitada e ordenada
através do DORM, que suporta cursores keyset (o cursor é assinado com a chave
configurada, pelo que não é falsificável — a gateway exige essa chave no `connect`,
sem predefinição insegura).

## Formato

Devolve a página mais um cursor `next` opaco; o cliente devolve-o para a página
seguinte. Mantém a chave de ordenação estável (um id monótono ou um tuplo
`(created_at, id)`) para que o cursor fique bem definido.

## Relacionado

- [Fazer consultas](data-layer/queries.md) · [Controladores e encaminhamento](transport/controllers-and-routing.md)

---

*Parte da [documentação do Seiva](../index.md).*
